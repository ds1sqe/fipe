//! Exhaustiveness by specialization of complete pattern rows. Guards do
//! not contribute, and limits never shrink a meaning's constructors.
use super::{
    format, vec, Entry, Error, ExprId, ExprKind, ItemKind, Literal, Origin,
    Pass, PatternKind, Span, String, Target, ToString, Type, TypeName,
    TypeReason, Vec,
};
use crate::ast::{Arm, Member, PatternId};

#[derive(Clone)]
enum Pattern {
    Any,
    Case(String, Vec<Self>),
    Gate(TypeName, Box<Self>),
    Partial,
}
use alloc::boxed::Box;

struct Case {
    name: String,
    fields: Vec<(String, Type)>,
}

impl Pass<'_> {
    pub(super) fn coverage(&mut self) {
        for (index, expr) in self.module.expressions.iter().enumerate() {
            let Some(ty) = self.typed.expr(ExprId(index)).cloned() else {
                continue;
            };
            match &expr.kind {
                ExprKind::Arms(arms) => self.cover_arms(arms, &ty, expr.span),
                ExprKind::Bind { pattern, value, .. } => {
                    let Some(ty) = self
                        .typed
                        .casts
                        .get(value.0)
                        .and_then(Option::as_ref)
                        .map(|c| &c.to)
                        .or_else(|| self.typed.expr(*value))
                    else {
                        continue;
                    };
                    let good = self.good_binding(*pattern, ty);
                    let pattern = self.cover_pattern(*pattern, ty);
                    if !good
                        && self
                            .uncovered(
                                &[vec![pattern]],
                                core::slice::from_ref(ty),
                            )
                            .is_some()
                    {
                        self.errors.push(Error { span: expr.span, reason: TypeReason::Limit {
                            message: String::from("binding pattern is not total or a good-side binding"),
                        }});
                    }
                }
                _ => (),
            }
        }
        self.cover_constructors();
    }

    fn cover_constructors(&mut self) {
        for entry in &self.module.entries {
            let Entry::Item(item) = entry else { continue };
            let ItemKind::Impl { members, .. } = &item.kind else {
                continue;
            };
            let arms: Vec<_> = members
                .iter()
                .filter_map(|member| {
                    if let crate::ast::ImplMember::Constructor { arm, .. } =
                        member
                    {
                        Some(arm.clone())
                    } else {
                        None
                    }
                })
                .collect();
            let Some(first) = arms.first() else { continue };
            let Some(types) = first
                .patterns
                .iter()
                .map(|p| self.typed.pattern(*p).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            self.cover_rows(&arms, &types, item.span);
        }
    }

    fn good_binding(&self, id: PatternId, ty: &Type) -> bool {
        if let PatternKind::Group(inner) = self.module.patterns[id.0].kind {
            return self.good_binding(inner, ty);
        }
        let Type::Named { name, .. } = crate::infer::meaning(ty) else {
            return false;
        };
        if name.origin != Origin::Prelude
            || !matches!(name.name.as_str(), "Maybe" | "Result")
        {
            return false;
        }
        let Pattern::Case(name, parts) = self.cover_pattern(id, ty) else {
            return false;
        };
        if !matches!(name.as_str(), "Just" | "Ok") {
            return false;
        }
        self.cover_cases(ty)
            .into_iter()
            .find(|c| c.name == name)
            .is_some_and(|c| {
                let types: Vec<_> =
                    c.fields.into_iter().map(|(_, t)| t).collect();
                self.uncovered(&[parts], &types).is_none()
            })
    }

    fn cover_arms(&mut self, arms: &[Arm], ty: &Type, span: Span) {
        let mut types = Vec::new();
        let mut ty = ty;
        for _ in 0..arms.first().map_or(0, |a| a.patterns.len()) {
            let Type::Function { parameter, result } = ty else {
                return;
            };
            types.push((**parameter).clone());
            ty = result;
        }
        self.cover_rows(arms, &types, span);
    }

    fn cover_rows(&mut self, arms: &[Arm], types: &[Type], span: Span) {
        if arms.iter().any(|a| a.patterns.len() != types.len()) {
            return;
        }
        // An outcome bracket whose family is unknown has its own error;
        // coverage is silent while it stands.
        if arms.iter().any(|a| {
            a.patterns
                .iter()
                .zip(types)
                .any(|(p, t)| self.bracket_without_family(*p, t))
        }) {
            return;
        }
        let rows: Vec<_> = arms
            .iter()
            .filter(|a| a.guard.is_none())
            .map(|a| {
                a.patterns
                    .iter()
                    .zip(types)
                    .map(|(p, t)| self.cover_pattern(*p, t))
                    .collect()
            })
            .collect();
        if let Some(missing) = self.uncovered(&rows, types) {
            self.errors.push(Error {
                span,
                reason: TypeReason::Limit {
                    message: format!(
                        "non-total arms: uncovered {}",
                        missing.join(" ")
                    ),
                },
            });
        }
    }

    fn bracket_without_family(&self, id: PatternId, ty: &Type) -> bool {
        let node = &self.module.patterns[id.0];
        match &node.kind {
            PatternKind::Group(p) => self.bracket_without_family(*p, ty),
            PatternKind::Outcome { .. } => {
                let cases = self.cover_cases(ty);
                !cases.iter().any(|c| {
                    matches!(c.name.as_str(), "Just" | "Nothing" | "Ok" | "Err")
                })
            }
            _ => false,
        }
    }

    fn cover_pattern(&self, id: PatternId, ty: &Type) -> Pattern {
        let node = &self.module.patterns[id.0];
        let cases = self.cover_cases(ty);
        match &node.kind {
            PatternKind::Group(p) => self.cover_pattern(*p, ty),
            PatternKind::Wildcard | PatternKind::Default { .. } => Pattern::Any,
            PatternKind::Name(n) if !n.is_uppercase() => Pattern::Any,
            PatternKind::Literal(Literal::Unit) => {
                Pattern::Case(String::from("()"), Vec::new())
            }
            PatternKind::Literal(_) => Pattern::Partial,
            PatternKind::Tuple(parts) | PatternKind::List(parts) => {
                let Some(case) = cases.first() else {
                    return Pattern::Partial;
                };
                if case.fields.len() != parts.len() {
                    return Pattern::Partial;
                }
                Pattern::Case(
                    case.name.clone(),
                    parts
                        .iter()
                        .zip(&case.fields)
                        .map(|(p, (_, t))| self.cover_pattern(*p, t))
                        .collect(),
                )
            }
            PatternKind::Outcome { good, payload } => {
                let name = match crate::infer::meaning(ty) {
                    Type::Named { name, .. } if name.name == "Maybe" => {
                        if *good {
                            "Just"
                        } else {
                            "Nothing"
                        }
                    }
                    _ => {
                        if *good {
                            "Ok"
                        } else {
                            "Err"
                        }
                    }
                };
                let Some(case) = cases.iter().find(|c| c.name == name) else {
                    return Pattern::Partial;
                };
                let parts = case
                    .fields
                    .iter()
                    .map(|(_, t)| {
                        payload
                            .map_or(Pattern::Any, |p| self.cover_pattern(p, t))
                    })
                    .collect();
                Pattern::Case(String::from(name), parts)
            }
            PatternKind::Name(_)
            | PatternKind::Constructor { .. }
            | PatternKind::Record { .. } => self.named_pattern(id, ty, &cases),
        }
    }

    fn named_pattern(
        &self,
        id: PatternId,
        ty: &Type,
        cases: &[Case],
    ) -> Pattern {
        let node = &self.module.patterns[id.0];
        let target = self.resolved.pattern(id);
        let (origin, owner, name) = match target {
            Some(Target::Type { origin, name }) => (origin, name, name),
            Some(Target::Constructor { origin, ty, name }) => {
                (origin, ty, name)
            }
            _ => return Pattern::Partial,
        };
        let payload = match node.kind {
            PatternKind::Constructor { payload, .. } => Some(payload),
            _ => None,
        };
        if matches!(crate::infer::meaning(ty), Type::Either(_)) {
            let key = format!("{origin:?}:{owner}");
            let Some(case) = cases.iter().find(|c| c.name == key) else {
                return Pattern::Partial;
            };
            return Pattern::Case(
                key,
                vec![payload.map_or(Pattern::Any, |p| {
                    self.cover_pattern(p, &case.fields[0].1)
                })],
            );
        }
        if matches!(target, Some(Target::Type { .. }))
            && !matches!(node.kind, PatternKind::Record { .. })
        {
            return Pattern::Gate(
                TypeName {
                    origin: origin.clone(),
                    name: name.clone(),
                },
                Box::new(
                    payload.map_or(Pattern::Any, |p| self.cover_pattern(p, ty)),
                ),
            );
        }
        let Some(case) = cases.iter().find(|c| &c.name == name) else {
            return Pattern::Partial;
        };
        let mut parts = vec![Pattern::Any; case.fields.len()];
        if let Some(p) = payload {
            if let Some((_, t)) = case.fields.first() {
                parts[0] = self.cover_pattern(p, t);
            }
        }
        if let PatternKind::Record { fields, .. } = &node.kind {
            for field in fields {
                if let Some(p) = field.pattern {
                    if let Some(i) = case.fields.iter().position(|(n, _)| {
                        Some(n) == field.name.segments.last()
                    }) {
                        parts[i] = self.cover_pattern(p, &case.fields[i].1);
                    }
                }
            }
        }
        Pattern::Case(case.name.clone(), parts)
    }

    fn cover_cases(&self, ty: &Type) -> Vec<Case> {
        self.cover_cases_at(ty, 0)
    }

    fn cover_cases_at(&self, ty: &Type, depth: usize) -> Vec<Case> {
        if depth > crate::parse::MAX_NESTING {
            return Vec::new();
        }
        match crate::infer::meaning(ty) {
            Type::Unit => vec![Case {
                name: String::from("()"),
                fields: Vec::new(),
            }],
            Type::Tuple(parts) => vec![Case {
                name: String::from("tuple"),
                fields: parts
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (i.to_string(), t.clone()))
                    .collect(),
            }],
            Type::Array { element, length } => {
                // Only written list lengths need expansion; an unmentioned
                // array is handled by the wildcard row without allocation.
                let count = self.module.patterns.iter().find_map(|p| match &p
                    .kind
                {
                    PatternKind::List(parts)
                        if **length
                            == Type::Number((parts.len() as u64).into()) =>
                    {
                        Some(parts.len())
                    }
                    _ => None,
                });
                count.map_or_else(Vec::new, |n| {
                    vec![Case {
                        name: String::from("array"),
                        fields: (0..n)
                            .map(|i| (i.to_string(), (**element).clone()))
                            .collect(),
                    }]
                })
            }
            Type::Either(parts) => parts
                .iter()
                .filter_map(|t| {
                    let Type::Named { name, .. } = crate::infer::meaning(t)
                    else {
                        return None;
                    };
                    Some(Case {
                        name: format!("{:?}:{}", name.origin, name.name),
                        fields: vec![(String::new(), t.clone())],
                    })
                })
                .collect(),
            Type::Named { name, arguments } => {
                self.named_cases(name, arguments, depth + 1)
            }
            Type::Both(parts) => parts
                .iter()
                .map(|p| self.cover_cases_at(p, depth + 1))
                .find(|c| !c.is_empty())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn named_cases(
        &self,
        name: &TypeName,
        arguments: &[Type],
        depth: usize,
    ) -> Vec<Case> {
        // Vec's representation is not its pattern space.
        if name.origin == Origin::Prelude && name.name == "Vec" {
            return Vec::new();
        }
        let Some(module) = self.owner(&name.origin) else {
            return Vec::new();
        };
        let Some(index) = self.declarations_by_name().get(name) else {
            return Vec::new();
        };
        let Entry::Item(item) = &module.entries[*index] else {
            return Vec::new();
        };
        let ItemKind::Type {
            parameters,
            members,
            base,
            ..
        } = &item.kind
        else {
            return Vec::new();
        };
        let bindings: Vec<_> = parameters
            .iter()
            .zip(arguments)
            .map(|(p, t)| {
                (
                    p.name.segments.last().cloned().unwrap_or_default(),
                    t.clone(),
                )
            })
            .collect();
        let Some(members) = members else {
            return base.map_or_else(Vec::new, |base| {
                self.cover_cases_at(
                    &self.syntax_bound(module, &name.origin, base, &bindings),
                    depth,
                )
            });
        };
        let field = |f: &crate::ast::Field| {
            (
                f.name.segments.last().cloned().unwrap_or_default(),
                self.syntax_bound(module, &name.origin, f.ty, &bindings),
            )
        };
        if members
            .iter()
            .any(|m| matches!(m, Member::Constructor { .. }))
        {
            members
                .iter()
                .filter_map(|m| match m {
                    Member::Constructor { name, fields, .. } => Some(Case {
                        name: name.segments.last().cloned().unwrap_or_default(),
                        fields: fields.iter().map(field).collect(),
                    }),
                    Member::Field(_) => None,
                })
                .collect()
        } else {
            // A record pattern over a private field cannot be written
            // outside its module, so a witness stops at the record.
            let hidden = !matches!(name.origin, Origin::This | Origin::Line(_))
                && members
                    .iter()
                    .any(|m| matches!(m, Member::Field(f) if !f.public));
            if hidden {
                return Vec::new();
            }
            vec![Case {
                name: name.name.clone(),
                fields: members
                    .iter()
                    .filter_map(|m| match m {
                        Member::Field(f) => Some(field(f)),
                        Member::Constructor { .. } => None,
                    })
                    .collect(),
            }]
        }
    }

    fn uncovered(
        &self,
        rows: &[Vec<Pattern>],
        types: &[Type],
    ) -> Option<Vec<String>> {
        if types.is_empty() {
            return rows.is_empty().then(Vec::new);
        }
        if rows
            .iter()
            .any(|r| r.iter().all(|p| matches!(p, Pattern::Any)))
        {
            return None;
        }
        if self.gates_cover(rows, types) {
            return None;
        }
        let cases = self.cover_cases(&types[0]);
        if !cases.is_empty() {
            for case in cases {
                let specialized: Vec<_> = rows
                    .iter()
                    .filter_map(|row| {
                        let mut head = match &row[0] {
                            Pattern::Any => {
                                vec![Pattern::Any; case.fields.len()]
                            }
                            Pattern::Case(n, parts) if *n == case.name => {
                                parts.clone()
                            }
                            _ => return None,
                        };
                        head.extend_from_slice(&row[1..]);
                        Some(head)
                    })
                    .collect();
                let mut fields: Vec<_> =
                    case.fields.iter().map(|(_, t)| t.clone()).collect();
                fields.extend_from_slice(&types[1..]);
                if let Some(mut witness) = self.uncovered(&specialized, &fields)
                {
                    let rest = witness.split_off(case.fields.len());
                    let label =
                        case.name.rsplit(':').next().unwrap_or(&case.name);
                    let head = if witness.is_empty() {
                        String::from(label)
                    } else {
                        match label {
                            "tuple" => format!("({})", witness.join(", ")),
                            "array" => format!("[{}]", witness.join(", ")),
                            _ => format!("{label} ({})", witness.join(", ")),
                        }
                    };
                    return Some(core::iter::once(head).chain(rest).collect());
                }
            }
            return None;
        }
        let rows: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r[0], Pattern::Any))
            .map(|r| r[1..].to_vec())
            .collect();
        self.uncovered(&rows, &types[1..])
            .map(|w| core::iter::once(String::from("_")).chain(w).collect())
    }
    fn gates_cover(&self, rows: &[Vec<Pattern>], types: &[Type]) -> bool {
        let gates: Vec<_> = rows
            .iter()
            .filter_map(|r| match &r[0] {
                Pattern::Gate(g, _) => Some(g.clone()),
                _ => None,
            })
            .collect();
        for gate in &gates {
            let opposite = self
                .declaration(gate)
                .filter(|(_, _, l)| l.is_empty())
                .and_then(|(m, b, _)| {
                    b.map(|b| self.syntax_type(m, &gate.origin, b))
                });
            if let Some(other) = opposite.as_ref().and_then(|b| {
                gates
                    .iter()
                    .find(|g| self.negated_gate(b, core::slice::from_ref(g)))
            }) {
                let covers = [gate, other].iter().all(|g| {
                    let rows: Vec<_> = rows
                        .iter()
                        .filter_map(|r| {
                            let mut row = match &r[0] {
                                Pattern::Any => vec![Pattern::Any],
                                Pattern::Gate(n, p) if n == *g => {
                                    vec![(**p).clone()]
                                }
                                Pattern::Gate(_, _) => return None,
                                p => vec![p.clone()],
                            };
                            row.extend_from_slice(&r[1..]);
                            Some(row)
                        })
                        .collect();
                    self.uncovered(&rows, types).is_none()
                });
                if covers {
                    return true;
                }
            }
        }

        false
    }
}
