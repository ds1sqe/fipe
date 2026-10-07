use fipe::{
    resolve::Origin,
    types::{Mismatch, Obligation, Scheme, Subst, Type, Var},
};

fn i64() -> Type {
    Type::named(Origin::Prelude, "I64", vec![])
}

fn vec(of: Type) -> Type {
    Type::named(Origin::Prelude, "Vec", vec![of])
}

#[test]
fn unification_binds_solves_and_rejects_with_the_first_difference() {
    let mut subst = Subst::default();
    let a = Type::Var(subst.fresh());
    subst.unify(&vec(a.clone()), &vec(vec(i64()))).unwrap();
    assert_eq!(subst.apply(&a), vec(i64()));

    let b = Type::Var(subst.fresh());
    assert_eq!(
        subst.unify(&b, &vec(b.clone())),
        Err(Box::new(Mismatch::Occurs {
            variable: Var(1),
            ty: vec(b.clone())
        }))
    );
    assert_eq!(
        subst.unify(&Type::Rigid("a".into()), &i64()),
        Err(Box::new(Mismatch::Different {
            expected: Type::Rigid("a".into()),
            actual: i64()
        }))
    );
    // Nominal identity is the declaration, not the spelling.
    let mine = Type::named(Origin::This, "Vec", vec![i64()]);
    assert!(subst.unify(&mine, &vec(i64())).is_err());
    assert_eq!(
        subst.unify(
            &Type::function(i64(), Type::Unit),
            &Type::function(i64(), i64())
        ),
        Err(Box::new(Mismatch::Different {
            expected: Type::Unit,
            actual: i64()
        }))
    );
    assert!(subst
        .unify(
            &Type::Array {
                element: Box::new(i64()),
                length: 4
            },
            &Type::Array {
                element: Box::new(i64()),
                length: 3
            }
        )
        .is_err());
}

#[test]
fn substitution_is_idempotent_and_unification_symmetric() {
    let cases: Vec<(Type, Type)> = {
        let mut subst = Subst::default();
        let vars: Vec<_> = (0..6).map(|_| Type::Var(subst.fresh())).collect();
        vec![
            (vars[0].clone(), vec(vars[1].clone())),
            (
                Type::function(vars[1].clone(), vars[2].clone()),
                Type::function(i64(), vars[3].clone()),
            ),
            (vec(vars[4].clone()), vars[5].clone()),
            (vars[5].clone(), vec(vec(i64()))),
        ]
    };
    let mut forward = Subst::default();
    let mut backward = Subst::default();
    for _ in 0..6 {
        forward.fresh();
        backward.fresh();
    }
    for (left, right) in &cases {
        assert_eq!(
            forward.unify(left, right).is_ok(),
            backward.unify(right, left).is_ok()
        );
    }
    // Unifying the same pairs in opposite orders can leave a different
    // unbound variable as each pair's representative (`Var(2)` vs
    // `Var(3)`, say), so the two substitutions need not agree on a
    // literal type for every pair; only within one substitution do
    // idempotence and the unified pairs' equality hold.
    for (left, right) in &cases {
        let forward_once = forward.apply(left);
        assert_eq!(forward.apply(&forward_once), forward_once);
        assert_eq!(forward.apply(left), forward.apply(right));

        let backward_once = backward.apply(left);
        assert_eq!(backward.apply(&backward_once), backward_once);
        assert_eq!(backward.apply(left), backward.apply(right));
    }
    assert_eq!(forward.apply(&Type::Var(Var(0))), vec(i64()));
    assert_eq!(backward.apply(&Type::Var(Var(0))), vec(i64()));
}

#[test]
fn schemes_generalize_over_the_environment_and_instantiate_fresh() {
    let mut subst = Subst::default();
    let a = subst.fresh();
    let b = subst.fresh();
    let ty = Type::function(Type::Var(a), Type::Var(b));
    let obligations = [Obligation::Member {
        name: "add".into(),
        head: Type::Var(a),
        ty: Type::function(Type::Var(a), Type::Var(a)),
        dot: fipe::token::Span::default(),
    }];
    let scheme = Scheme::generalize(&ty, &obligations, &[b], &subst);
    assert_eq!(scheme.variables, [a]);
    assert_eq!(scheme.to_string(), "a.add: a -> a => a -> b");

    let identity = Scheme::generalize(
        &Type::function(Type::Var(a), Type::Var(a)),
        &[],
        &[],
        &subst,
    );
    assert_eq!(identity.to_string(), "a -> a");
    let (first, _) = identity.instantiate(&mut subst);
    let (second, _) = identity.instantiate(&mut subst);
    let mut vars = Vec::new();
    first.variables(&mut vars);
    second.variables(&mut vars);
    assert_eq!(vars.len(), 2);
    assert_ne!(first, second);
    subst.unify(&first, &Type::function(i64(), i64())).unwrap();
    assert!(matches!(subst.apply(&second), Type::Function { .. }));
    assert_ne!(subst.apply(&second), subst.apply(&first));

    let add = Scheme::generalize(
        &Type::function(
            Type::Var(a),
            Type::function(Type::Var(a), Type::Var(a)),
        ),
        &obligations,
        &[],
        &subst,
    );
    assert_eq!(add.to_string(), "a.add: a -> a => a -> a -> a");
    assert_eq!(
        Type::function(Type::function(i64(), i64()), vec(Type::Unit))
            .to_string(),
        "(I64 -> I64) -> Vec<()>"
    );
    assert_eq!(Scheme::mono(i64()).to_string(), "I64");
}

#[test]
fn shadowed_type_names_show_their_origins() {
    let a = Type::named(Origin::Line(0), "A", vec![]);
    let b = Type::named(Origin::Line(2), "A", vec![]);
    assert_ne!(a.beside(&b).to_string(), b.beside(&a).to_string());
}

#[test]
fn variables_bind_meanings_without_carrying_place_limits() {
    let integer = Type::named(Origin::Prelude, "Integer", vec![]);
    let limited = Type::Limited {
        name: Box::new(Type::named(Origin::Prelude, "U8", vec![])),
        meaning: Box::new(integer.clone()),
    };
    for (place, meaning) in [
        (limited.clone(), integer.clone()),
        (vec(limited.clone()), vec(integer.clone())),
        (
            Type::function(limited.clone(), limited),
            Type::function(integer.clone(), integer),
        ),
    ] {
        let mut subst = Subst::default();
        let variable = Type::Var(subst.fresh());
        subst.unify(&variable, &place).unwrap();
        assert_eq!(subst.apply(&variable), meaning);
    }
}
