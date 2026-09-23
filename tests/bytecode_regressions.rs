use fipe::bytecode_vm::bytecode::{
    instruction::Instruction, instructions::Instructions,
};

#[test]
fn reads_operand_immediately_after_opcode() {
    let mut code = Instructions::create().unwrap();
    let offset = code
        .add_instruction(Instruction::CONST { idx: 42 })
        .unwrap();
    assert_eq!(
        code.read_instruction(offset).unwrap(),
        Instruction::CONST { idx: 42 }
    );
}

#[test]
fn index_preserves_the_following_instruction() {
    let mut code = Instructions::create().unwrap();
    let index = code.add_instruction(Instruction::INDEX).unwrap();
    let next = code
        .add_instruction(Instruction::CONST { idx: 42 })
        .unwrap();
    assert_eq!(code.read_instruction(index).unwrap(), Instruction::INDEX);
    assert_eq!(
        code.read_instruction(next).unwrap(),
        Instruction::CONST { idx: 42 }
    );
}

#[test]
fn grows_beyond_initial_capacity_without_losing_instructions() {
    let mut code = Instructions::create().unwrap();
    let mut offsets = Vec::new();
    for idx in 0..512 {
        offsets.push(code.add_instruction(Instruction::CONST { idx }).unwrap());
    }
    for (idx, offset) in offsets.into_iter().enumerate() {
        assert_eq!(
            code.read_instruction(offset).unwrap(),
            Instruction::CONST { idx }
        );
    }
}

#[test]
fn modifying_a_clone_preserves_the_original() {
    let mut original = Instructions::create().unwrap();
    let offset = original.add_instruction(Instruction::ADD).unwrap();
    let mut copy = original.clone();
    // Both instructions occupy one byte, satisfying the update contract.
    unsafe { copy.update_instruction(Instruction::SUB, offset) };
    assert_eq!(original.read_instruction(offset).unwrap(), Instruction::ADD);
    assert_eq!(copy.read_instruction(offset).unwrap(), Instruction::SUB);
}

#[test]
fn reads_closure_operands_at_every_word_offset() {
    for padding in 0..std::mem::size_of::<usize>() {
        let mut code = Instructions::create().unwrap();
        for _ in 0..padding {
            code.add_instruction(Instruction::ADD).unwrap();
        }
        let expected = Instruction::CLOSURE { idx: 1234, free: 7 };
        let offset = code.add_instruction(expected.clone()).unwrap();
        assert_eq!(code.read_instruction(offset).unwrap(), expected);
    }
}

#[test]
fn rejects_missing_or_truncated_operands() {
    let mut code = Instructions::create().unwrap();
    assert!(code.read_instruction(0).is_err());
    assert!(code.read_instruction(usize::MAX).is_err());
    code.add_instruction(Instruction::CONST { idx: 42 })
        .unwrap();
    code.remove_instruction(1);
    assert!(code.read_instruction(0).is_err());
}
