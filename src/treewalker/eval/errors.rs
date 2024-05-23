use crate::{
    ast::Expression,
    immix::errors::ImmixError,
    object::{Object, ObjectType},
    token::Kind,
};

#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    BlankResult,
    EnvironmentHasDropped,

    LetStatementValueIsNone,
    EvaluationOfExpressionIsNone(Expression),
    LeftExpressionIsNone,
    RightExpressionIsNone,

    NotABoolean(Object),
    NotAFunction(Object),

    ConditionIsNone,
    FunctionIsNone,
    ElementIsNone,
    ArrayIsNone,

    IdentifierNotFound(String),

    NotSameType,
    NotArray,

    IndexIsNotAInt(Object),
    IndexIsNegative(Object),
    IndexOutOfRange(IndexErrorDetail),

    FunctionArgLengthNotMatched(ArgumentsLength),

    DivideWithZero,

    InvalidPrefixOperationTarget(ObjectType, Kind),
    InvalidInfixOperationTarget(ObjectType, Kind),

    InvalidStringInfixOperation(Kind),

    InvalidIntegerInfixOperation(Kind),
    InvalidIntegerPrefixOperation(Kind),

    InvalidBoolInfixOperation(Kind),
    InvalidBoolPrefixOperation(Kind),

    MemoryError(ImmixError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArgumentsLength {
    pub function_args: usize,
    pub called_with: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexErrorDetail {
    pub array_length: usize,
    pub called_with: usize,
}

impl From<ImmixError> for EvalError {
    fn from(value: ImmixError) -> Self {
        Self::MemoryError(value)
    }
}
