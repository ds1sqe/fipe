use std::fmt::Debug;

use crate::token;

#[derive(Debug, Clone)]
pub enum Node {
    Program(Program),
    Statement(Statement),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    ExpressionStatement(ExpressionStatement),
    LetStatement(LetStatement),
    ReturnStatement(ReturnStatement),
    BlockStatement(BlockStatement),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    Identifier(Identifier),
    IntegerLiteral(IntegerLiteral),
    BooleanLiteral(BooleanLiteral),
    StringLiteral(StringLiteral),
    FunctionLiteral(FunctionLiteral),
    ArrayLiteral(ArrayLiteral),
    InfixExpression(InfixExpression),
    PrefixExpression(PrefixExpression),
    IfExpression(IfExpression),
    CallExpression(CallExpression),
    IndexExpression(IndexExpression),
}

pub trait Nodetrait {
    fn literal(&self) -> String;
    fn to_str(&self) -> String;
    fn to_node(self) -> Node;
}

impl Nodetrait for Statement {
    fn literal(&self) -> String {
        match self {
            Statement::LetStatement(stm) => stm.literal(),
            Statement::ExpressionStatement(stm) => stm.literal(),
            Statement::ReturnStatement(stm) => stm.literal(),
            Statement::BlockStatement(stm) => stm.literal(),
        }
    }

    fn to_str(&self) -> String {
        match self {
            Statement::LetStatement(stm) => stm.to_str(),
            Statement::ExpressionStatement(stm) => stm.to_str(),
            Statement::ReturnStatement(stm) => stm.to_str(),
            Statement::BlockStatement(stm) => stm.to_str(),
        }
    }

    fn to_node(self) -> Node {
        Node::Statement(self)
    }
}

impl Nodetrait for Expression {
    fn literal(&self) -> String {
        match self {
            Expression::Identifier(idt) => idt.literal(),
            Expression::IntegerLiteral(int) => int.literal(),
            Expression::BooleanLiteral(bool) => bool.literal(),
            Expression::FunctionLiteral(flit) => flit.literal(),
            Expression::StringLiteral(slit) => slit.literal(),
            Expression::ArrayLiteral(alit) => alit.literal(),
            Expression::InfixExpression(ifix) => ifix.literal(),
            Expression::PrefixExpression(pfix) => pfix.literal(),
            Expression::IfExpression(ifx) => ifx.literal(),
            Expression::CallExpression(cexp) => cexp.literal(),
            Expression::IndexExpression(idx) => idx.literal(),
        }
    }

    fn to_str(&self) -> String {
        match self {
            Expression::Identifier(idt) => idt.to_str(),
            Expression::IntegerLiteral(int) => int.to_str(),
            Expression::BooleanLiteral(bool) => bool.to_str(),
            Expression::FunctionLiteral(flit) => flit.to_str(),
            Expression::StringLiteral(slit) => slit.to_str(),
            Expression::ArrayLiteral(alit) => alit.to_str(),
            Expression::InfixExpression(ifix) => ifix.to_str(),
            Expression::PrefixExpression(pfix) => pfix.to_str(),
            Expression::IfExpression(ifx) => ifx.to_str(),
            Expression::CallExpression(cexp) => cexp.to_str(),
            Expression::IndexExpression(idx) => idx.to_str(),
        }
    }

    fn to_node(self) -> Node {
        Node::Expression(self)
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>,
}
impl Program {
    pub fn new() -> Self {
        Program {
            statements: Vec::new(),
        }
    }
    pub fn push_stm(&mut self, stm: Statement) {
        self.statements.push(stm);
    }
}

impl Nodetrait for Program {
    fn literal(&self) -> String {
        let mut buf = String::new();

        buf.push_str("Program: ");

        for stm in self.statements.iter() {
            buf.push_str(&stm.literal())
        }

        buf
    }

    fn to_str(&self) -> String {
        let mut buf = String::new();

        buf.push_str("Program: ");

        for stm in self.statements.iter() {
            buf.push_str(&stm.to_str())
        }

        buf
    }

    fn to_node(self) -> Node {
        Node::Program(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Identifier {
    pub token: token::Token, // token::IDENT
    pub value: String,
}

impl Nodetrait for Identifier {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        self.value.clone()
    }

    fn to_node(self) -> Node {
        Expression::Identifier(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntegerLiteral {
    pub token: token::Token, // token::Int
    pub value: i64,
}

impl Nodetrait for IntegerLiteral {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        self.value.clone().to_string()
    }
    fn to_node(self) -> Node {
        Expression::IntegerLiteral(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BooleanLiteral {
    pub token: token::Token, // token::False or True
    pub value: bool,
}

impl Nodetrait for BooleanLiteral {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        self.value.clone().to_string()
    }
    fn to_node(self) -> Node {
        Expression::BooleanLiteral(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StringLiteral {
    pub token: token::Token, // token::False or True
    pub value: String,
}

impl Nodetrait for StringLiteral {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        self.value.clone().to_string()
    }
    fn to_node(self) -> Node {
        Expression::StringLiteral(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionLiteral {
    pub token: token::Token, // token::Function
    pub ident: Option<Identifier>,
    pub parameters: Vec<Identifier>,
    pub body: BlockStatement,
}

impl Nodetrait for FunctionLiteral {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut params = Vec::new();
        for param in &self.parameters {
            params.push(param.to_str())
        }
        let mut buf = String::new();
        buf.push_str(&self.literal());
        if self.ident.is_some() {
            buf.push_str(" ");
            buf.push_str(&self.ident.as_ref().unwrap().to_str());
        }
        buf.push_str("(");
        buf.push_str(&params.join(", "));
        buf.push_str(") {");
        buf.push_str(&self.body.to_str());
        buf.push_str("}");
        buf
    }
    fn to_node(self) -> Node {
        Expression::FunctionLiteral(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayLiteral {
    pub elements: Vec<Expression>,
}

impl Nodetrait for ArrayLiteral {
    fn literal(&self) -> String {
        "Array".to_string()
    }
    fn to_str(&self) -> String {
        let mut elements = Vec::new();
        for el in &self.elements {
            elements.push(el.to_str())
        }

        let mut buf = String::new();
        buf.push_str("[");
        buf.push_str(&elements.join(", "));
        buf.push_str("]");
        buf
    }
    fn to_node(self) -> Node {
        Expression::ArrayLiteral(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LetStatement {
    pub token: token::Token,
    pub identifier: Identifier,
    pub value: Option<Expression>,
}

impl Nodetrait for LetStatement {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.literal());
        buf.push_str(" ");
        buf.push_str(&self.identifier.to_str());

        if self.value.is_some() {
            buf.push_str(" = ");
            buf.push_str(&self.value.as_ref().unwrap().to_str())
        }
        buf.push_str(";");
        buf
    }
    fn to_node(self) -> Node {
        Statement::LetStatement(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStatement {
    pub token: token::Token,
    pub value: Option<Expression>,
}

impl Nodetrait for ReturnStatement {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.literal());
        if self.value.is_some() {
            buf.push_str(" ");
            buf.push_str(&self.value.as_ref().unwrap().to_str())
        }
        buf.push_str(";");
        buf
    }
    fn to_node(self) -> Node {
        Statement::ReturnStatement(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockStatement {
    pub token: token::Token,
    pub statements: Vec<Statement>,
}

impl Nodetrait for BlockStatement {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        for statement in &self.statements {
            buf.push_str(&statement.to_str());
        }
        buf
    }
    fn to_node(self) -> Node {
        Statement::BlockStatement(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpressionStatement {
    pub token: token::Token,
    pub expression: Option<Expression>,
}
impl Nodetrait for ExpressionStatement {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str("");
        if self.expression.is_some() {
            buf.push_str(&self.expression.as_ref().unwrap().to_str());
        }
        buf
    }
    fn to_node(self) -> Node {
        Statement::ExpressionStatement(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrefixExpression {
    pub token: token::Token,
    pub right: Box<Expression>,
}
impl Nodetrait for PrefixExpression {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.literal());
        buf.push_str("(");
        buf.push_str(&self.right.to_str());
        buf.push_str(")");
        buf
    }
    fn to_node(self) -> Node {
        Expression::PrefixExpression(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InfixExpression {
    pub token: token::Token,
    pub left: Box<Expression>,
    pub operator: token::Token,
    pub right: Box<Expression>,
}
impl Nodetrait for InfixExpression {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str("(");
        buf.push_str(&self.left.to_str());
        buf.push_str(" ");
        buf.push_str(&self.operator.literal);
        buf.push_str(" ");
        buf.push_str(&self.right.to_str());
        buf.push_str(")");
        buf
    }
    fn to_node(self) -> Node {
        Expression::InfixExpression(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfExpression {
    pub token: token::Token, // Token::If
    pub condition: Box<Expression>,
    pub consequence: BlockStatement,
    pub alternative: Option<BlockStatement>,
}
impl Nodetrait for IfExpression {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str("if ");
        buf.push_str(&self.condition.to_str());
        buf.push_str(" {");
        buf.push_str(&self.consequence.to_str());
        buf.push_str("}");

        if self.alternative.is_some() {
            buf.push_str(" else {");
            buf.push_str(&self.alternative.as_ref().unwrap().to_str());
            buf.push_str("}");
        }
        buf
    }
    fn to_node(self) -> Node {
        Expression::IfExpression(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub token: token::Token, // Token::IDENT
    pub function: Box<Expression>,
    pub arguments: Vec<Expression>,
}
impl Nodetrait for CallExpression {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();

        let mut args = Vec::new();
        for arg in &self.arguments {
            args.push(arg.to_str())
        }

        buf.push_str(&self.function.to_str());
        buf.push_str("(");
        buf.push_str(&args.join(", "));
        buf.push_str(")");
        buf
    }
    fn to_node(self) -> Node {
        Expression::CallExpression(self).to_node()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpression {
    pub token: token::Token, // Token::LBRACKET
    pub left: Box<Expression>,
    pub index: Box<Expression>,
}
impl Nodetrait for IndexExpression {
    fn literal(&self) -> String {
        self.token.literal.clone()
    }
    fn to_str(&self) -> String {
        let mut buf = String::new();
        buf.push_str(&self.left.to_str());
        buf.push_str("[");
        buf.push_str(&self.index.to_str());
        buf.push_str("]");
        buf
    }
    fn to_node(self) -> Node {
        Expression::IndexExpression(self).to_node()
    }
}
