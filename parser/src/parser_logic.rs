use super::errors::{ParsingError, UnclosedCommentError};
use super::lang;
use program_structure::ast::AST;
use program_structure::report::Report;
use program_structure::file_definition::FileID;

pub fn preprocess(expr: &str, file_id: FileID) -> Result<String, Report> {
    let mut pp = String::new();
    let mut state = 0;
    let mut loc = 0;
    let mut block_start = 0;

    let mut it = expr.chars();
    while let Some(c0) = it.next() {
        loc += 1;
        match (state, c0) {
            (0, '/') => {
                loc += 1;
                match it.next() {
                    Some('/') => {
                        state = 1;
                        pp.push(' ');
                        pp.push(' ');
                    }
                    Some('*') => {
                        block_start = loc;
                        state = 2;
                        pp.push(' ');
                        pp.push(' ');
                    }
                    Some(c1) => {
                        pp.push(c0);
                        pp.push(c1);
                    }
                    None => {
                        pp.push(c0);
                        break;
                    }
                }
            }
            (0, _) => pp.push(c0),
            (1, '\n') => {
                pp.push(c0);
                state = 0;
            }
            (2, '*') => {
                loc += 1;
                match it.next() {
                    Some('/') => {
                        pp.push(' ');
                        pp.push(' ');
                        state = 0;
                    }
                    Some(c) => {
                        pp.push(' ');
                        for _i in 0..c.len_utf8() {
                            pp.push(' ');
                        }
                    }
                    None => {
                        let error =
                            UnclosedCommentError { location: block_start..block_start, file_id };
                        return Err(UnclosedCommentError::produce_report(error));
                    }
                }
            }
            (_, c) => {
                for _i in 0..c.len_utf8() {
                    pp.push(' ');
                }
            }
        }
    }
    Ok(pp)
}

pub fn parse_file(src: &str, file_id: FileID) -> Result<AST, Report> {
    use lalrpop_util::ParseError::*;
    lang::ParseAstParser::new()
        .parse(&preprocess(src, file_id)?)
        .map(|mut ast| {
            // Set file ID for better error reporting.
            for include in &mut ast.includes {
                include.meta.set_file_id(file_id);
            }
            ast
        })
        .map_err(|parse_error| match parse_error {
            InvalidToken { location } => ParsingError {
                file_id,
                msg: format!("{:?}", parse_error),
                location: location..location,
            },
            UnrecognizedToken { ref token, .. } => ParsingError {
                file_id,
                msg: format!("{:?}", parse_error),
                location: token.0..token.2,
            },
            ExtraToken { ref token } => ParsingError {
                file_id,
                msg: format!("{:?}", parse_error),
                location: token.0..token.2,
            },
            _ => ParsingError { file_id, msg: format!("{:?}", parse_error), location: 0..0 },
        })
        .map_err(ParsingError::produce_report)
}

pub fn parse_string(src: &str) -> Option<AST> {
    let src = preprocess(src, 0).ok()?;
    lang::ParseAstParser::new().parse(&src).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_string;

    #[test]
    fn test_parse_string() {
        let function = r#"
            function f(m) {
                // This is a comment.
                var x = 1024;
                var y = 16;
                while (x < m) {
                    x += y;
                }
                if (x == m) {
                    x = 0;
                }
                /* This is another comment. */
                return x;
            }
        "#;
        let _ = parse_string(function);

        let template = r#"
            template T(m) {
                signal input in[m];
                signal output out;

                var sum = 0;
                for (var i = 0; i < m; i++) {
                    sum += in[i];
                }
                out <== sum;
            }
        "#;
        let _ = parse_string(template);
    }
}
