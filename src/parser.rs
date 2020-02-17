use crate::ast::{ASTNode, CmdEvalArgs, GroupType, VAN};

// === parser utils

fn get_offset_of(whole_buffer: &str, part: &str) -> usize {
    // NOTE: use offset_from() once it's stable
    part.as_ptr() as usize - whole_buffer.as_ptr() as usize
}

fn str_slice_between<'a>(whole_buffer_start: &'a str, post_part: &'a str) -> &'a str {
    &whole_buffer_start[..get_offset_of(whole_buffer_start, post_part)]
}

fn is_scope_end(x: char) -> bool {
    match x {
        /* '(' */ ')' => true,
        /* '{' */ '}' => true,
        _ => false,
    }
}

fn astnode_is_space(x: &ASTNode) -> bool {
    match x {
        ASTNode::NullNode | ASTNode::Constant(false, _) => true,
        _ => false,
    }
}

/// 1. part while f(x) == true, then 2. part
fn str_split_at_while(x: &str, mut f: impl FnMut(char) -> bool) -> (&str, &str) {
    x.split_at(
        x.chars()
            .take_while(|i| f(*i))
            .map(|i| i.len_utf8())
            .sum::<usize>(),
    )
}

/// escaped escape symbol or other escaped code: optional passthrough
fn parse_escaped_const(i: char, opts: ParserOptions) -> Option<ASTNode> {
    Some(ASTNode::Constant(
        true,
        match i {
            _ if i == opts.escc && !opts.pass_escc => {
                let mut tmp = [0; 4];
                let tmp = opts.escc.encode_utf8(&mut tmp);
                (*tmp).into()
            }
            '\n' => crulst_atom!(""),
            '$' => crulst_atom!("$"),
            _ => return None,
        },
    ))
}

fn str_split_at_ctrl(
    data: &str,
    opts: ParserOptions,
    f_do_cont_at: impl Fn(char) -> bool,
) -> (&str, &str) {
    str_split_at_while(data, |i| match i {
        '$' | '(' | ')' | '{' | '}' => false,
        _ if i == opts.escc => false,
        _ => f_do_cont_at(i),
    })
}

pub(crate) fn args2unspaced(args: VAN) -> CmdEvalArgs {
    use crate::mangle_ast::MangleAST;
    use itertools::Itertools;
    args.into_iter()
        .group_by(astnode_is_space)
        .into_iter()
        .filter(|(d, _)| !*d)
        .map(|(_, i)| i.collect::<VAN>().lift_ast().simplify())
        .collect()
}

// === parser options

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ParserOptions {
    escc: char,
    pass_escc: bool,
}

impl ParserOptions {
    #[inline]
    pub fn new(escc: char, pass_escc: bool) -> Self {
        Self { escc, pass_escc }
    }
}

// === parse trait

trait Parse: Sized {
    /// # Return value
    /// * `Ok(rest, parsed_obj)`
    /// * `Err(offending_code, description)`
    fn parse(data: &str, opts: ParserOptions) -> Result<(&str, Self), (&str, &'static str)>;
}

impl Parse for ASTNode {
    fn parse(data: &str, opts: ParserOptions) -> Result<(&str, Self), (&str, &'static str)> {
        let escc = opts.escc;
        let mut iter = data.chars();

        let i = iter.next().ok_or_else(|| (data, "unexpected EOF"))?;
        match i {
            _ if i == escc => {
                let d_after_escc = iter.as_str();
                let i = iter.next().ok_or_else(|| (data, "unexpected EOF"))?;
                if i == '(' {
                    // got begin of cmdeval block
                    let (rest, mut vanx) = VAN::parse(iter.as_str(), opts)?;
                    if vanx.is_empty() {
                        return Err((&data[..std::cmp::min(data.len(), 3)], "got empty eval stmt"));
                    }
                    let mut iter = rest.chars();
                    if iter.next() != Some(')') {
                        return Err((data, "expected ')' instead"));
                    }

                    // extract command
                    assert!(!vanx.is_empty());
                    let split_point = vanx
                        .iter()
                        .enumerate()
                        .filter_map(|y| {
                            if astnode_is_space(&y.1) {
                                Some(y.0 + 1)
                            } else {
                                None
                            }
                        })
                        .next()
                        .unwrap_or(1);
                    let van = vanx.split_off(split_point);
                    let mut cmd = vanx;
                    if cmd.last().map(astnode_is_space).unwrap() {
                        cmd.pop();
                    }
                    Ok((iter.as_str(), ASTNode::CmdEval(cmd, args2unspaced(van))))
                } else if is_scope_end(i) {
                    Err((
                        str_slice_between(data, iter.as_str()),
                        "dangerous escaped end-of-scope marker",
                    ))
                } else if let Some(c) = parse_escaped_const(i, opts) {
                    Ok((iter.as_str(), c))
                } else {
                    // interpret it as a command (LaTeX-alike)
                    let (cmd, mut rest) =
                        str_split_at_ctrl(d_after_escc, opts, |x| !x.is_whitespace());
                    if cmd.is_empty() {
                        return Err((
                            str_slice_between(data, iter.as_str()),
                            "got empty eval stmt",
                        ));
                    }
                    let vanx = if rest.starts_with('(') {
                        let (tmp_rest, tmp) = ASTNode::parse(rest, opts)?;
                        if let ASTNode::Grouped(GroupType::Strict, van) = tmp {
                            rest = tmp_rest;
                            args2unspaced(van)
                        } else {
                            unreachable!()
                        }
                    } else {
                        Default::default()
                    };
                    Ok((
                        rest,
                        ASTNode::CmdEval(vec![ASTNode::Constant(true, cmd.into())], vanx),
                    ))
                }
            }
            '(' => {
                let (rest, van) = VAN::parse(iter.as_str(), opts)?;
                let mut iter = rest.chars();
                if iter.next() != Some(')') {
                    return Err((rest, /* '(' */ "expected ')' instead"));
                }
                Ok((iter.as_str(), ASTNode::Grouped(GroupType::Strict, van)))
            }
            '{' => {
                let (rest, van) = VAN::parse(iter.as_str(), opts)?;
                let mut iter = rest.chars();
                if iter.next() != Some('}') {
                    return Err((rest, /* '{' */ "expected '}' instead"));
                }
                Ok((iter.as_str(), ASTNode::Grouped(GroupType::Loose, van)))
            }
            '$' => {
                let (cdat, rest) = str_split_at_while(iter.as_str(), |i| i == '$');
                let (idxs, rest) = str_split_at_while(rest, |i| i.is_digit(10));
                Ok((
                    rest,
                    ASTNode::Argument {
                        indirection: cdat.len(),
                        index: idxs.parse().ok(),
                    },
                ))
            }
            _ if is_scope_end(i) => Err((
                str_slice_between(data, iter.as_str()),
                "unexpected unbalanced end-of-scope marker",
            )),
            _ => {
                let is_whitespace = i.is_whitespace();
                let (cdat, rest) =
                    str_split_at_ctrl(data, opts, |x| x.is_whitespace() == is_whitespace);
                Ok((rest, ASTNode::Constant(!is_whitespace, cdat.into())))
            }
        }
    }
}

impl Parse for VAN {
    fn parse(mut data: &str, opts: ParserOptions) -> Result<(&str, Self), (&str, &'static str)> {
        let mut ret = VAN::new();
        while data.chars().next().map(is_scope_end) == Some(false) {
            let (rest, node) = ASTNode::parse(data, opts)?;
            ret.push(node);
            data = rest;
        }
        Ok((data, ret))
    }
}

// === main parser

/// At top level, only parse things inside CmdEval's
fn parse_toplevel(
    mut data: &str,
    opts: ParserOptions,
) -> Result<(&str, VAN), (&str, &'static str)> {
    let mut ret = VAN::new();
    while !data.is_empty() {
        let mut cstp_has_nws = false;
        let (cstp, rest) = str_split_at_while(data, |i| {
            cstp_has_nws |= !i.is_whitespace();
            i != opts.escc
        });
        if !cstp.is_empty() {
            ret.push(ASTNode::Constant(cstp_has_nws, cstp.into()));
        }
        data = if !rest.is_empty() {
            let (rest, node) = ASTNode::parse(rest, opts)?;
            ret.push(node);
            rest
        } else {
            rest
        };
    }
    Ok((data, ret))
}

pub fn file2ast(filename: &str, opts: ParserOptions) -> Result<VAN, anyhow::Error> {
    use anyhow::Context;

    let fh = readfilez::read_from_file(std::fs::File::open(filename))
        .with_context(|| format!("unable to read file '{}'", filename))?;
    let input = std::str::from_utf8(fh.as_slice())
        .with_context(|| format!("file '{}' contains non-UTF-8 data", filename))?;

    use anyhow::anyhow;
    match parse_toplevel(input, opts) {
        Ok((rest, van)) => {
            if rest.is_empty() {
                Ok(van)
            } else {
                Err(anyhow!(
                    "unexpected EOF (more closing parens as opening parens)"
                ))
            }
        }
        Err((offending, descr)) => {
            use codespan_reporting::{
                diagnostic::{Diagnostic, Label},
                term,
            };
            use std::{convert::TryFrom, str::FromStr};

            let writer = term::termcolor::StandardStream::stderr(
                term::ColorArg::from_str("auto").unwrap().into(),
            );
            let config = term::Config::default();
            let mut files = codespan::Files::new();
            let fileid = files.add(filename, input);
            let start_pos = u32::try_from(get_offset_of(input, offending)).unwrap();
            let offending_len = u32::try_from(offending.len()).unwrap();

            term::emit(
                &mut writer.lock(),
                &config,
                &files,
                &Diagnostic::new_error(
                    descr.to_string(),
                    Label::new(fileid, start_pos..(start_pos + offending_len), ""),
                ),
            )
            .unwrap();
            Err(anyhow!("{}", descr))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args2unspaced() {
        use ASTNode::*;
        assert_eq!(
            args2unspaced(vec![
                Constant(true, "a".into()),
                Constant(false, "a".into()),
                Constant(true, "a".into()),
                Constant(true, "a".into()),
                Constant(false, "a".into())
            ]),
            CmdEvalArgs(vec![
                Constant(true, "a".into()),
                Constant(true, "aa".into())
            ])
        );
    }
}
