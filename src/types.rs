//! The type representation of [types.md]: types, schemes, substitution
//! and unification. Nothing here reads syntax; inference does.
//!
//! [types.md]: https://github.com/ds1sqe/fipe/blob/develop/Documentation/spec/types.md

use crate::{resolve::Origin, token::Span};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;

/// An inference variable, an index into a [`Subst`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Var(pub u32);

/// A declared type's identity: where it was declared and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeName {
    pub origin: Origin,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Unit,
    Var(Var),
    /// A rigid variable, written in an annotation.
    Rigid(String),
    Named {
        name: TypeName,
        arguments: Vec<Self>,
    },
    Array {
        element: Box<Self>,
        length: u64,
    },
    Function {
        parameter: Box<Self>,
        result: Box<Self>,
    },
}

impl Type {
    #[must_use]
    pub fn named(origin: Origin, name: &str, arguments: Vec<Self>) -> Self {
        Self::Named {
            name: TypeName {
                origin,
                name: String::from(name),
            },
            arguments,
        }
    }

    #[must_use]
    pub fn function(parameter: Self, result: Self) -> Self {
        Self::Function {
            parameter: Box::new(parameter),
            result: Box::new(result),
        }
    }

    /// The inference variables in this type, in order of appearance.
    pub fn variables(&self, out: &mut Vec<Var>) {
        match self {
            Self::Unit | Self::Rigid(_) => {}
            Self::Var(var) => {
                if !out.contains(var) {
                    out.push(*var);
                }
            }
            Self::Named { arguments, .. } => {
                for argument in arguments {
                    argument.variables(out);
                }
            }
            Self::Array { element, .. } => element.variables(out),
            Self::Function { parameter, result } => {
                parameter.variables(out);
                result.variables(out);
            }
        }
    }

    /// Whether `var` occurs in this type.
    #[must_use]
    pub fn contains(&self, var: Var) -> bool {
        let mut vars = Vec::new();
        self.variables(&mut vars);
        vars.contains(&var)
    }
}

/// What a scheme owes: a trait on a type, `Num a`, or a member of a
/// type not yet known, `a.name: t`, found at the dot `dot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Obligation {
    Class {
        class: String,
        ty: Type,
    },
    Member {
        head: Type,
        name: String,
        ty: Type,
        dot: Span,
    },
}

impl Obligation {
    /// The type the obligation is on: the class's type, the member's head.
    #[must_use]
    pub const fn subject(&self) -> &Type {
        match self {
            Self::Class { ty, .. } | Self::Member { head: ty, .. } => ty,
        }
    }

    /// The same obligation with `f` applied to each of its types.
    #[must_use]
    pub fn map(&self, mut f: impl FnMut(&Type) -> Type) -> Self {
        match self {
            Self::Class { class, ty } => Self::Class {
                class: class.clone(),
                ty: f(ty),
            },
            Self::Member {
                head,
                name,
                ty,
                dot,
            } => Self::Member {
                head: f(head),
                name: name.clone(),
                ty: f(ty),
                dot: *dot,
            },
        }
    }

    /// The inference variables of its types, in order of appearance.
    pub fn variables(&self, out: &mut Vec<Var>) {
        match self {
            Self::Class { ty, .. } => ty.variables(out),
            Self::Member { head, ty, .. } => {
                head.variables(out);
                ty.variables(out);
            }
        }
    }
}

/// `∀ variables. obligations ⇒ ty`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub variables: Vec<Var>,
    pub obligations: Vec<Obligation>,
    pub ty: Type,
}

impl Scheme {
    /// A type quantified over nothing.
    #[must_use]
    pub const fn mono(ty: Type) -> Self {
        Self {
            variables: Vec::new(),
            obligations: Vec::new(),
            ty,
        }
    }

    /// Quantify over every variable of the applied type and obligations
    /// that is not free in the environment (BIND).
    #[must_use]
    pub fn generalize(
        ty: &Type,
        obligations: &[Obligation],
        environment: &[Var],
        subst: &Subst,
    ) -> Self {
        let ty = subst.apply(ty);
        let obligations: Vec<_> = obligations
            .iter()
            .map(|obligation| obligation.map(|ty| subst.apply(ty)))
            .collect();
        let mut variables = Vec::new();
        ty.variables(&mut variables);
        for obligation in &obligations {
            obligation.variables(&mut variables);
        }
        variables.retain(|var| !environment.contains(var));
        Self {
            variables,
            obligations,
            ty,
        }
    }

    /// Replace every quantified variable with a fresh one (VAR).
    #[must_use]
    pub fn instantiate(&self, subst: &mut Subst) -> (Type, Vec<Obligation>) {
        let fresh: Vec<_> = self
            .variables
            .iter()
            .map(|var| (*var, Type::Var(subst.fresh())))
            .collect();
        let ty = replace(&self.ty, &fresh);
        let obligations = self
            .obligations
            .iter()
            .map(|obligation| obligation.map(|ty| replace(ty, &fresh)))
            .collect();
        (ty, obligations)
    }
}

/// `ty` with each listed variable replaced.
pub(crate) fn replace(ty: &Type, fresh: &[(Var, Type)]) -> Type {
    match ty {
        Type::Var(var) => fresh
            .iter()
            .find(|(old, _)| old == var)
            .map_or_else(|| ty.clone(), |(_, new)| new.clone()),
        Type::Unit | Type::Rigid(_) => ty.clone(),
        Type::Named { name, arguments } => Type::Named {
            name: name.clone(),
            arguments: arguments.iter().map(|a| replace(a, fresh)).collect(),
        },
        Type::Array { element, length } => Type::Array {
            element: Box::new(replace(element, fresh)),
            length: *length,
        },
        Type::Function { parameter, result } => {
            Type::function(replace(parameter, fresh), replace(result, fresh))
        }
    }
}

/// Why two types did not unify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mismatch {
    Different {
        expected: Type,
        actual: Type,
    },
    /// `a` against a type containing `a`: no finite solution.
    Occurs {
        variable: Var,
        ty: Type,
    },
}

/// The solved variables so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Subst {
    bindings: Vec<Option<Type>>,
}

impl Subst {
    /// A substitution over `bindings`, one per variable from `Var(0)`.
    #[must_use]
    pub(crate) const fn from_bindings(bindings: Vec<Option<Type>>) -> Self {
        Self { bindings }
    }

    /// Each variable's solution, from `Var(0)`.
    #[must_use]
    pub(crate) fn bindings(&self) -> &[Option<Type>] {
        &self.bindings
    }

    /// How many variables it has made.
    #[must_use]
    pub(crate) fn count(&self) -> u32 {
        u32::try_from(self.bindings.len()).unwrap_or(u32::MAX)
    }

    pub fn fresh(&mut self) -> Var {
        self.bindings.push(None);
        Var(u32::try_from(self.bindings.len() - 1).unwrap_or(u32::MAX))
    }

    /// The type with every solved variable replaced, to a fixed point.
    #[must_use]
    pub fn apply(&self, ty: &Type) -> Type {
        match ty {
            Type::Var(var) => self
                .get(*var)
                .map_or_else(|| ty.clone(), |bound| self.apply(bound)),
            Type::Unit | Type::Rigid(_) => ty.clone(),
            Type::Named { name, arguments } => Type::Named {
                name: name.clone(),
                arguments: arguments.iter().map(|a| self.apply(a)).collect(),
            },
            Type::Array { element, length } => Type::Array {
                element: Box::new(self.apply(element)),
                length: *length,
            },
            Type::Function { parameter, result } => {
                Type::function(self.apply(parameter), self.apply(result))
            }
        }
    }

    fn get(&self, var: Var) -> Option<&Type> {
        self.bindings.get(var.0 as usize).and_then(Option::as_ref)
    }

    /// Make `expected` and `actual` equal, binding variables; nominal
    /// names must match by declaration identity, rigid variables only
    /// themselves.
    ///
    /// # Errors
    /// The first pair that cannot be made equal, after substitution.
    pub fn unify(
        &mut self,
        expected: &Type,
        actual: &Type,
    ) -> Result<(), Box<Mismatch>> {
        let expected = self.apply(expected);
        let actual = self.apply(actual);
        match (&expected, &actual) {
            (Type::Var(a), Type::Var(b)) if a == b => Ok(()),
            (Type::Var(var), other) | (other, Type::Var(var)) => {
                if other.contains(*var) {
                    return Err(Box::new(Mismatch::Occurs {
                        variable: *var,
                        ty: other.clone(),
                    }));
                }
                self.bindings[var.0 as usize] = Some(other.clone());
                Ok(())
            }
            (Type::Unit, Type::Unit) => Ok(()),
            (Type::Rigid(a), Type::Rigid(b)) if a == b => Ok(()),
            (
                Type::Named {
                    name: a,
                    arguments: x,
                },
                Type::Named {
                    name: b,
                    arguments: y,
                },
            ) if a == b && x.len() == y.len() => {
                for (x, y) in x.iter().zip(y) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (
                Type::Array {
                    element: a,
                    length: n,
                },
                Type::Array {
                    element: b,
                    length: m,
                },
            ) if n == m => self.unify(a, b),
            (
                Type::Function {
                    parameter: a,
                    result: x,
                },
                Type::Function {
                    parameter: b,
                    result: y,
                },
            ) => {
                self.unify(a, b)?;
                self.unify(x, y)
            }
            _ => Err(Box::new(Mismatch::Different { expected, actual })),
        }
    }
}

/// A type in the spec's syntax; its inference variables print as `a`,
/// `b`, … in order of appearance.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut variables = Vec::new();
        self.variables(&mut variables);
        write(self, f, &letters(&variables))
    }
}

/// A scheme in the spec's syntax: `Num a => a -> a -> a`, or
/// `a.age: b => b -> a -> Bool` for a member obligation, its quantified
/// variables named `a`, `b`, … in order, and any variable it does not
/// quantify by the letters after them.
impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut variables = self.variables.clone();
        for obligation in &self.obligations {
            obligation.variables(&mut variables);
        }
        self.ty.variables(&mut variables);
        let names = letters(&variables);
        for (index, obligation) in self.obligations.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            match obligation {
                Obligation::Class { class, ty } => {
                    write!(f, "{class} ")?;
                    write(ty, f, &names)?;
                }
                Obligation::Member { head, name, ty, .. } => {
                    write(head, f, &names)?;
                    write!(f, ".{name}: ")?;
                    write(ty, f, &names)?;
                }
            }
        }
        if !self.obligations.is_empty() {
            f.write_str(" => ")?;
        }
        write(&self.ty, f, &names)
    }
}

fn letters(variables: &[Var]) -> Vec<(Var, String)> {
    variables
        .iter()
        .enumerate()
        .map(|(index, var)| (*var, letter(index)))
        .collect()
}

fn letter(index: usize) -> String {
    let mut name = String::new();
    let letter = u8::try_from(index % 26).unwrap_or(0) + b'a';
    name.push(char::from(letter));
    if index >= 26 {
        write_number(&mut name, index / 26);
    }
    name
}

fn write_number(out: &mut String, number: usize) {
    use fmt::Write as _;
    let _ = write!(out, "{number}");
}

fn write(
    ty: &Type,
    f: &mut fmt::Formatter<'_>,
    names: &[(Var, String)],
) -> fmt::Result {
    match ty {
        Type::Unit => f.write_str("()"),
        Type::Var(var) => match names.iter().find(|(v, _)| v == var) {
            Some((_, name)) => f.write_str(name),
            // Every variable is named before writing.
            None => f.write_str("_"),
        },
        Type::Rigid(name) => f.write_str(name),
        Type::Named { name, arguments } => {
            f.write_str(&name.name)?;
            if arguments.is_empty() {
                return Ok(());
            }
            f.write_str("<")?;
            for (index, argument) in arguments.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write(argument, f, names)?;
            }
            f.write_str(">")
        }
        Type::Array { element, length } => {
            f.write_str("[")?;
            write(element, f, names)?;
            write!(f, "; {length}]")
        }
        Type::Function { parameter, result } => {
            let grouped = matches!(**parameter, Type::Function { .. });
            if grouped {
                f.write_str("(")?;
            }
            write(parameter, f, names)?;
            if grouped {
                f.write_str(")")?;
            }
            f.write_str(" -> ")?;
            write(result, f, names)
        }
    }
}
