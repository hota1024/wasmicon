enum Item {
    Comment(String),
    InlineComment(String),
    Label {
        name: String,
    },
    Op {
        opcode: String,
        operands: Vec<Operand>,
    },
}

#[derive(Clone)]
pub enum Operand {
    SP,
    RegA(usize),
    Imm(i32),
    Symbol(String),
    LiteralI32(i32),
}

impl ToString for Operand {
    fn to_string(&self) -> String {
        match self {
            SP => "sp".to_string(),
            RegA(i) => format!("a{}", i),
            Imm(i) => format!("{}", i),
            Symbol(s) => s.clone(),
            LiteralI32(v) => format!("{}", v),
        }
    }
}

pub use Operand::*;

pub struct AsmWriter {
    items: Vec<Item>,
}

impl AsmWriter {
    pub fn new() -> Self {
        AsmWriter { items: vec![] }
    }

    pub fn op<T: ToString>(&mut self, opcode: T, operands: Vec<Operand>) -> &mut Self {
        self.items.push(Item::Op {
            opcode: opcode.to_string(),
            operands,
        });

        self
    }

    pub fn label<T: ToString>(&mut self, name: T) -> &mut Self {
        self.items.push(Item::Label {
            name: name.to_string(),
        });

        self
    }

    pub fn comment<T: ToString>(&mut self, comment: T) -> &mut Self {
        self.items.push(Item::Comment(comment.to_string()));

        self
    }

    pub fn inline_comment<T: ToString>(&mut self, comment: T) -> &mut Self {
        self.items.push(Item::InlineComment(comment.to_string()));

        self
    }

    pub fn write_to_string(&self) -> String {
        let mut s = String::new();

        let mut p = self.items.iter().peekable();

        loop {
            let item = match p.next() {
                Some(item) => item,
                None => break,
            };

            match item {
                Item::Comment(comment) => {
                    s.push_str(&format!("\t# {}", comment));
                }
                Item::InlineComment(comment) => {
                    s.push_str(&format!("\t# {}", comment));
                }
                Item::Label { name } => {
                    s.push_str(&format!("{}:", name));
                }
                Item::Op { opcode, operands } => {
                    let operands_str = operands
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    s.push_str(&format!("\t{}\t{}", opcode, operands_str));
                }
            }

            if !matches!(p.peek(), Some(Item::InlineComment(_))) {
                s.push('\n');
            }
        }

        s
    }

    pub fn extend(&mut self, other: AsmWriter) {
        self.items.extend(other.items);
    }
}

pub fn symbol<T: ToString>(s: T) -> Operand {
    Operand::Symbol(s.to_string())
}
