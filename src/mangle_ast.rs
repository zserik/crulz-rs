use crate::ast::{ASTNode, CmdEvalArgs, GroupType, LiftAST, VAN};
use delegate_attr::delegate;
use itertools::Itertools;

// do NOT "use ASTNode::*;" here, because sometimes we want to "use ASTNodeClass::*;"

pub trait MangleAST: Default {
    fn to_str(self, escc: char) -> String;

    /// helper for MangleAST::simplify and interp::eval
    fn get_complexity(&self) -> usize;

    #[inline(always)]
    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    #[inline]
    fn simplify_inplace(&mut self) {
        *self = self.take().simplify();
    }

    /// this cleanup up the AST, opposite of two lift_ast invocations
    fn simplify(self) -> Self;

    /// this apply_arguments function applies the 'args' to the AST
    /// # Return value
    /// * `Err(idx)`: the first applied index which wasn't present in 'args'
    fn apply_arguments_inplace(&mut self, args: &CmdEvalArgs) -> Result<(), usize>;
}

impl MangleAST for ASTNode {
    fn to_str(self, escc: char) -> String {
        use ASTNode::*;
        match self {
            NullNode => String::new(),
            Constant(_, x) => x.to_string(),
            Grouped(gt, elems) => {
                let inner = elems.to_str(escc);
                if gt == GroupType::Strict {
                    format!("({})", inner)
                } else {
                    inner
                }
            }
            Argument { indirection, index } => std::iter::repeat('$')
                .take(indirection + 1)
                .chain(
                    index
                        .as_ref()
                        .map(usize::to_string)
                        .iter()
                        .flat_map(|i| i.chars()),
                )
                .collect(),
            CmdEval(cmd, args) => format!("{}({}{})", escc, cmd.to_str(escc), args.to_str(escc)),
        }
    }

    fn get_complexity(&self) -> usize {
        use ASTNode::*;
        match &self {
            NullNode => 0,
            Constant(_, x) => 1 + x.len(),
            Argument { indirection, .. } => 3 + indirection,
            Grouped(gt, x) => {
                (match *gt {
                    GroupType::Dissolving => 0,
                    GroupType::Loose => 1,
                    GroupType::Strict => 2,
                }) + x.get_complexity()
            }
            CmdEval(cmd, x) => 1 + cmd.get_complexity() + x.get_complexity(),
        }
    }

    fn simplify(mut self) -> Self {
        use ASTNode::*;
        let mut cplx = self.get_complexity();
        loop {
            match &mut self {
                Grouped(ref mut gt, ref mut x) => {
                    match x.len() {
                        0 => {
                            if *gt != GroupType::Strict {
                                self = NullNode;
                            }
                        }
                        1 => {
                            let y = x[0].take().simplify();
                            if *gt != GroupType::Strict {
                                self = y;
                            } else if let Grouped(GroupType::Dissolving, z) = y {
                                *x = z;
                            } else {
                                // swap it back, omit clone
                                x[0] = y;
                            }
                        }
                        _ => x.simplify_inplace(),
                    }
                }
                CmdEval(ref mut cmd, ref mut args) => {
                    cmd.simplify_inplace();
                    args.simplify_inplace();
                }
                _ => break,
            }
            let new_cplx = self.get_complexity();
            if new_cplx >= cplx {
                break;
            }
            cplx = new_cplx;
        }
        self
    }

    fn apply_arguments_inplace(&mut self, xargs: &CmdEvalArgs) -> Result<(), usize> {
        use ASTNode::*;
        match self {
            Argument {
                indirection: 0,
                index,
            } => {
                *self = match *index {
                    Some(index) => match xargs.0.get(index) {
                        Some(x) => x.clone(),
                        None => return Err(index),
                    },
                    None => Constant(true, crulst_atom!("$")),
                };
            }
            Argument {
                ref mut indirection,
                ..
            } => *indirection -= 1,

            Grouped(_, ref mut x) => x.apply_arguments_inplace(xargs)?,
            CmdEval(ref mut cmd, ref mut args) => {
                cmd.apply_arguments_inplace(xargs)?;
                args.apply_arguments_inplace(xargs)?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl MangleAST for VAN {
    fn to_str(self, escc: char) -> String {
        self.into_iter()
            .fold(String::new(), |acc, i| acc + &i.to_str(escc))
    }

    #[inline]
    fn get_complexity(&self) -> usize {
        self.iter().map(|i| i.get_complexity()).sum()
    }

    fn simplify(self) -> Self {
        #[derive(PartialEq)]
        enum ASTNodeClass {
            NullNode,
            Constant(bool),
            Grouped(GroupType),
            Opaque,
        }

        self.into_iter()
            .map(|i| i.simplify())
            .group_by(|i| {
                use ASTNodeClass::*;
                match i {
                    ASTNode::Grouped(gt, x) if x.is_empty() && *gt != GroupType::Strict => NullNode,
                    ASTNode::Constant(_, x) if x.is_empty() => NullNode,
                    ASTNode::Constant(s, _) => Constant(*s),
                    ASTNode::Grouped(s, _) => Grouped(*s),
                    ASTNode::Argument { .. } | ASTNode::CmdEval(_, _) => Opaque,
                    _ => NullNode,
                }
            })
            .into_iter()
            .filter(|(d, _)| *d != ASTNodeClass::NullNode)
            .flat_map(|(d, i)| {
                use ASTNode::*;
                match d {
                    ASTNodeClass::Constant(x) => Constant(
                        x,
                        i.map(|j| {
                            if let Constant(_, y) = j {
                                y
                            } else {
                                unreachable!()
                            }
                        })
                        .fold(String::new(), |acc, i| acc + &i)
                        .into(),
                    )
                    .lift_ast(),
                    ASTNodeClass::Grouped(GroupType::Dissolving) => i
                        .flat_map(|j| {
                            if let Grouped(_, x) = j {
                                x
                            } else {
                                unreachable!()
                            }
                        })
                        .collect(),
                    _ => i.collect(),
                }
            })
            .collect()
    }

    fn apply_arguments_inplace(&mut self, args: &CmdEvalArgs) -> Result<(), usize> {
        for i in self.iter_mut() {
            i.apply_arguments_inplace(args)?;
        }
        Ok(())
    }
}

impl MangleAST for CmdEvalArgs {
    fn to_str(self, escc: char) -> String {
        self.0
            .into_iter()
            .fold(String::new(), |acc, i| acc + " " + &i.to_str(escc))
    }

    fn simplify(self) -> Self {
        self.into_iter()
            .map(|i| i.simplify())
            .flat_map(|i| {
                if let ASTNode::Grouped(GroupType::Dissolving, elems) = i {
                    elems
                } else {
                    i.lift_ast()
                }
            })
            .collect()
    }

    #[delegate(self.0)]
    fn get_complexity(&self) -> usize;

    #[delegate(self.0)]
    fn apply_arguments_inplace(&mut self, args: &CmdEvalArgs) -> Result<(), usize>;
}

pub trait MangleASTExt: MangleAST {
    fn compact_toplevel(self) -> Self;
}

impl MangleASTExt for VAN {
    fn compact_toplevel(self) -> Self {
        // we are at the top level, wo can inline non-strict groups
        // and then put all constants heaps into single constants
        self.into_iter()
            // 1. inline non-strict groups
            .flat_map(|i| match i {
                ASTNode::NullNode => vec![],
                ASTNode::Grouped(gt, x) if gt != GroupType::Strict => x.compact_toplevel(),
                _ => vec![i],
            })
            // 2. aggressive concat constant-after-constants
            .peekable()
            .batching(|it| {
                let (mut risp, mut rdat) = match it.next()? {
                    ASTNode::Constant(isp, dat) => (isp, dat.to_string()),
                    x => return Some(x),
                };
                while let Some(ASTNode::Constant(isp, ref dat)) = it.peek() {
                    risp |= isp;
                    rdat += &dat;
                    it.next();
                }
                Some(ASTNode::Constant(risp, rdat.into()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ASTNode::*;
    use super::*;

    #[test]
    fn test_simplify() {
        let ast = vec![
            Constant(true, "a".into()),
            Constant(true, "b".into())
                .lift_ast()
                .lift_ast()
                .lift_ast()
                .lift_ast(),
            Constant(true, "c".into()),
        ]
        .lift_ast()
        .lift_ast()
        .lift_ast();
        assert_eq!(ast.simplify(), Constant(true, "abc".into()));
    }

    #[test]
    fn test_compact_tl() {
        let ast = vec![
            Constant(true, "a".into()),
            Constant(false, "b".into()),
            Constant(true, "c".into()),
        ]
        .compact_toplevel();
        assert_eq!(ast, vec![Constant(true, "abc".into())]);
    }
}
