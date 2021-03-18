use lazy_static::lazy_static;
use regex::bytes::Regex;
use std::collections::HashMap;

pub struct Program {
    tt: TokenTree,
    renames: HashMap<Vec<u8>, Vec<u8>>,
}

impl Program {
    pub fn parse(code: &[u8]) -> Program {
        let tt = parse(code);
        let (tt, renames) = find_renames(tt);
        Program { tt, renames }
    }

    pub fn serialize(&self, ws: u8) -> Vec<u8> {
        let tt = apply_renames(&self.tt, &self.renames);
        let tt = apply_transform_to_load(tt, ws);
        serialize(&tt, ws)
    }
}

fn find_renames(mut tt: TokenTree) -> (TokenTree, HashMap<Vec<u8>, Vec<u8>>) {
    let mut renames = HashMap::new();
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^--\s*rename\s*(\w+)\s*->\s*(\w+)\s*$").unwrap();
    }
    tt.retain(|tok| {
        if let &TreeToken::Token {
            type_: TokenType::Comment,
            ref text,
        } = tok
        {
            if let Some(caps) = RE.captures(text) {
                renames.insert(caps[1].to_vec(), caps[2].to_vec());
                return false;
            }
        }
        true
    });
    (tt, renames)
}

fn apply_renames(tt: &TokenTree, renames: &HashMap<Vec<u8>, Vec<u8>>) -> TokenTree {
    let mut new_tt = vec![];

    for token in tt {
        match *token {
            TreeToken::Token {
                type_: TokenType::Identifier,
                ref text,
            } => {
                if let Some(new_name) = renames.get(text) {
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: new_name.clone(),
                    });
                } else {
                    new_tt.push(token.clone());
                }
            }
            TreeToken::Token { .. } => {
                new_tt.push(token.clone());
            }
            TreeToken::SubTree(ref sub_tt) => {
                new_tt.push(TreeToken::SubTree(apply_renames(sub_tt, renames)));
            }
        }
    }

    new_tt
}

fn apply_transform_to_load(tt: TokenTree, ws: u8) -> TokenTree {
    let mut new_tt = vec![];

    let mut transform_next = false;
    for token in tt {
        match token {
            TreeToken::Token {
                type_: TokenType::Comment,
                ref text,
            } => {
                lazy_static! {
                    static ref RE: Regex = Regex::new(r"^--\s*transform\s*to\s*load\s*$").unwrap();
                }
                if RE.is_match(text) {
                    transform_next = true;
                } else {
                    new_tt.push(token);
                }
            }
            TreeToken::Token { .. } => new_tt.push(token),
            TreeToken::SubTree(ref sub_tt) => {
                let func_name = if transform_next && sub_tt.len() >= 5 {
                    if sub_tt[0].text() == b"function"
                        && !sub_tt[1].text().is_empty()
                        && sub_tt[2].text() == b"("
                        && sub_tt[3].text() == b")"
                        && sub_tt[sub_tt.len() - 1].text() == b"end"
                    {
                        Some(sub_tt[1].text())
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(name) = func_name {
                    let body = serialize(&sub_tt[4..(sub_tt.len() - 1)], ws);
                    let mut string = vec![b'"'];
                    for c in body {
                        if c == b'"' {
                            string.push(b'\\');
                        }
                        string.push(c);
                    }
                    string.push(b'"');
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: name.to_vec(),
                    });
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Other,
                        text: b"=".to_vec(),
                    });
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: b"load".to_vec(),
                    });
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Other,
                        text: string,
                    });
                    transform_next = false;
                } else {
                    new_tt.push(token);
                }
            }
        }
    }

    new_tt
}

fn flatten(tokens: &mut Vec<(TokenType, Vec<u8>)>, tt: &[TreeToken]) {
    for token in tt {
        match *token {
            TreeToken::Token { type_, ref text } => tokens.push((type_, text.clone())),
            TreeToken::SubTree(ref sub_tt) => flatten(tokens, sub_tt),
        }
    }
}

fn serialize(tt: &[TreeToken], ws: u8) -> Vec<u8> {
    let mut tokens = vec![];
    flatten(&mut tokens, &tt);

    let mut code = vec![];
    let (mut last_token_type, mut last_token_text): (TokenType, &[u8]) = (TokenType::Other, &[]);
    for &(token_type, ref token_text) in &tokens {
        if token_type == TokenType::Comment {
            continue;
        }
        match last_token_type {
            TokenType::Identifier
                if token_text[0] == b'_' || token_text[0].is_ascii_alphanumeric() =>
            {
                code.push(ws);
            }
            TokenType::Number
                if token_text[0] == b'.'
                    || token_text[0].is_ascii_hexdigit()
                    || (token_text[0].to_ascii_lowercase() == b'x'
                        && (last_token_text == b"0" || last_token_text == b".0")) =>
            {
                code.push(ws);
            }
            TokenType::HexNumber
                if token_text[0] == b'.'
                    || token_text[0].is_ascii_hexdigit()
                    || token_text[0].to_ascii_lowercase() == b'p' =>
            {
                code.push(ws);
            }
            _ => (),
        }
        code.extend_from_slice(token_text);
        last_token_type = token_type;
        last_token_text = token_text.as_slice();
    }
    code
}

fn parse(mut code: &[u8]) -> TokenTree {
    fn parse_subtree(tokens: &mut TokenTree, code: &mut &[u8]) {
        loop {
            let (token_type, token_text) = next_token(code);
            if token_type == TokenType::EOF {
                return;
            }
            if token_type == TokenType::Identifier {
                if token_text == b"function" {
                    let mut sub_tokens = vec![];
                    sub_tokens.push(TreeToken::Token {
                        type_: token_type,
                        text: token_text.to_vec(),
                    });
                    parse_subtree(&mut sub_tokens, code);
                    tokens.push(TreeToken::SubTree(sub_tokens));
                    continue;
                }
                if token_text == b"do" || token_text == b"if" {
                    tokens.push(TreeToken::Token {
                        type_: token_type,
                        text: token_text.to_vec(),
                    });
                    parse_subtree(tokens, code);
                    continue;
                }
            }
            tokens.push(TreeToken::Token {
                type_: token_type,
                text: token_text.to_vec(),
            });
            if token_type == TokenType::Identifier && token_text == b"end" {
                return;
            }
        }
    }
    let mut tokens = vec![];
    parse_subtree(&mut tokens, &mut code);

    tokens
}

#[derive(Debug, Clone)]
enum TreeToken {
    Token { type_: TokenType, text: Vec<u8> },
    SubTree(TokenTree),
}

impl TreeToken {
    fn text(&self) -> &[u8] {
        if let &TreeToken::Token { ref text, .. } = self {
            text
        } else {
            b""
        }
    }
}

type TokenTree = Vec<TreeToken>;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
enum TokenType {
    Comment,
    Identifier,
    Number,
    HexNumber,
    EOF,
    Other,
}

fn next_token<'a>(code: &mut &'a [u8]) -> (TokenType, &'a [u8]) {
    lazy_static! {
        static ref WHITE_SPACE: Regex = Regex::new(r"\A\s+").unwrap();
        static ref LONG_BRACKET_COMMENT: Regex = Regex::new(r"\A--\[=*\[").unwrap();
        static ref COMMENT: Regex = Regex::new(r"\A--.*").unwrap();
        static ref IDENTIFIER: Regex = Regex::new(r"\A[_a-zA-Z][_a-zA-Z0-9]*").unwrap();
        static ref NUMBER: Regex = Regex::new(r"\A(\d+(\.\d*)?|\.\d+)([eE]-?\d+)?").unwrap();
        static ref HEXNUMBER: Regex =
            Regex::new(r"\A0[xX][[:xdigit:]]*(\.[[:xdigit:]]*)?([pP]-?\d+)?").unwrap();
        static ref LONG_BRACKET: Regex = Regex::new(r"\A\[=*\[").unwrap();
    }

    if let Some(m) = WHITE_SPACE.find(code) {
        *code = &code[m.end()..];
    }

    if let Some(m) = LONG_BRACKET_COMMENT.find(code) {
        let len = find_long_bracket_end(code, m.end() - 4);
        let string = &code[..len];
        *code = &code[len..];
        return (TokenType::Comment, string);
    }

    if let Some(m) = COMMENT.find(code) {
        *code = &code[m.end()..];
        return (TokenType::Comment, m.as_bytes());
    }

    if let Some(m) = IDENTIFIER.find(code) {
        *code = &code[m.end()..];
        return (TokenType::Identifier, m.as_bytes());
    }

    if let Some(m) = HEXNUMBER.find(code) {
        *code = &code[m.end()..];
        return (TokenType::HexNumber, m.as_bytes());
    }

    if let Some(m) = NUMBER.find(code) {
        *code = &code[m.end()..];
        return (TokenType::Number, m.as_bytes());
    }

    if code.len() > 0 {
        if code[0] == b'"' || code[0] == b'\'' {
            let delim = code[0];
            let mut pos = 1;
            while pos < code.len() {
                let c = code[pos];
                pos += 1;
                if c == delim {
                    break;
                }
                if c == b'\\' && pos < code.len() {
                    pos += 1;
                }
            }
            let string = &code[..pos];
            *code = &code[pos..];
            return (TokenType::Other, string);
        }
    }

    if let Some(m) = LONG_BRACKET.find(code) {
        let len = find_long_bracket_end(code, m.end() - 2);
        let string = &code[..len];
        *code = &code[len..];
        return (TokenType::Other, string);
    }

    if code.len() > 0 {
        let tok = &code[..1];
        *code = &code[1..];
        return (TokenType::Other, tok);
    }

    return (TokenType::EOF, b"");
}

fn find_long_bracket_end(code: &[u8], level: usize) -> usize {
    let mut p = 0;
    while p + level + 2 < code.len() {
        if code[p] == b']'
            && code[p + 1 + level] == b']'
            && (0..level).all(|o| code[p + 1 + o] == b'=')
        {
            break;
        }
        p += 1;
    }
    p + 2 + level
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn multiline_strings() {
        let mut input: &[u8] = b"[==[foo[=[bar]=]baz]==]...";
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"[==[foo[=[bar]=]baz]==]");
    }

    #[test]
    fn strings() {
        let mut input: &[u8] = b"\"test\\\"a\\\"\"  'foo\\''";
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"\"test\\\"a\\\"\"");
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"'foo\\''");
    }

    fn transform(code: &[u8]) -> Vec<u8> {
        Program::parse(code).serialize(b' ')
    }

    #[test]
    fn number_spaces() {
        assert_eq!(transform(b"ad=0x3FF9 poke(ad,r)"), b"ad=0x3FF9 poke(ad,r)");
        assert_eq!(transform(b"ad=0x3FF9 x=1"), b"ad=0x3FF9x=1");
        assert_eq!(transform(b"ad=0x3FF9 f=1"), b"ad=0x3FF9 f=1");
        assert_eq!(transform(b"ad=0x3FF9.2 p=1"), b"ad=0x3FF9.2 p=1");
        assert_eq!(transform(b"ad=0x3FF9.2p4 p=1"), b"ad=0x3FF9.2p4 p=1");
        assert_eq!(transform(b"ad=0x3FF9.2p-4 p=1"), b"ad=0x3FF9.2p-4 p=1");

        assert_eq!(transform(b"a=1 p=2"), b"a=1p=2");
        assert_eq!(transform(b"a=1 e=2"), b"a=1 e=2");
        assert_eq!(transform(b"a=0 x=2"), b"a=0 x=2");
        assert_eq!(transform(b"a=.0 x=2"), b"a=.0 x=2");
    }

    #[test]
    fn strings_spaces() {
        assert_eq!(
            transform(b"a=\" a=2 b=3 \\\" \\ c=4 d=5 \" b=2"),
            b"a=\" a=2 b=3 \\\" \\ c=4 d=5 \"b=2"
        );
        assert_eq!(
            transform(b"a=' a=2 b=3 \\' \\ c=4 d=5 ' b=2"),
            b"a=' a=2 b=3 \\' \\ c=4 d=5 'b=2"
        );
        assert_eq!(
            transform(b"a=[==[ this is ]=] fun ]==] b = 2"),
            b"a=[==[ this is ]=] fun ]==]b=2"
        );
    }

    #[test]
    fn multiline_comments() {
        assert_eq!(transform(b"a = --[=[ blah \n blub ]=] 4"), b"a=4");
    }
}
