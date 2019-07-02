use crate::llparser::Sections;
use crate::sharpen::classify_as_vec;
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum ASTNode {
    NullNode,

    /// Constant: is_non_space, data
    Constant(bool, Vec<u8>),

    /// Grouped: is_strict, elems
    /// loose groups are created while replacing patterns
    Grouped(bool, Box<Vec<ASTNode>>),

    CmdEval(String, Box<Vec<ASTNode>>),
}

// do NOT "use ASTNode::*;" here, because sometimes we want to "use ASTNodeClass::*;"
pub type VAN = Vec<ASTNode>;

impl std::default::Default for ASTNode {
    #[inline]
    fn default() -> Self {
        ASTNode::NullNode
    }
}

impl ASTNode {
    pub fn as_constant(&self) -> Option<&Vec<u8>> {
        match &self {
            ASTNode::Constant(_, x) => Some(x),
            _ => None,
        }
    }
}

macro_rules! crossparse {
    ($fn:path, $input:expr, $escc:ident) => {{
        // we don't want to import this in every file using this macro
        // but we use it in this file too, and want to suppress the
        // warning about that
        #[allow(unused_imports)]
        use crate::hlparser::ToAST;
        $fn($input, $escc).to_ast($escc)
    }};
}

pub trait ToAST {
    fn to_ast(self, escc: u8) -> VAN;
}

impl ToAST for Sections {
    fn to_ast(self, escc: u8) -> VAN {
        let mut top = Vec::<ASTNode>::new();

        for (is_cmdeval, section) in self {
            assert!(!section.is_empty());
            let slen = section.len();
            use crate::llparser::{parse_whole, IsSpace};
            if is_cmdeval {
                let first_space = section.iter().position(|&x| x.is_space());
                let rest = first_space.map(|x| &section[x + 1..]).unwrap_or(&[]);

                top.push(ASTNode::CmdEval(
                    std::str::from_utf8(&section[0..first_space.unwrap_or(slen)])
                        .expect("got non-utf8 symbol")
                        .to_owned(),
                    Box::new(crossparse!(parse_whole, rest, escc)),
                ));
            } else if section[0] == 40 && *section.last().unwrap() == 41 {
                top.push(ASTNode::Grouped(
                    true,
                    Box::new(crossparse!(parse_whole, &section[1..slen - 1], escc)),
                ));
            } else {
                top.par_extend(
                    classify_as_vec(section, |i| i.is_space())
                        .into_par_iter()
                        .map(|(ccl, x)| ASTNode::Constant(!ccl, x)),
                );
            }
        }

        top
    }
}
