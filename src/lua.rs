use lazy_static::lazy_static;
use regex::bytes::Regex;
use std::collections::HashMap;

struct Input<'a> {
    code: &'a [u8],
    pos: usize,
}

impl<'a> Input<'a> {
    fn new(code: &[u8]) -> Input {
        Input { code, pos: 0 }
    }
    fn next(&self) -> u8 {
        if self.pos >= self.code.len() {
            0
        } else {
            self.code[self.pos]
        }
    }
    fn step(&mut self) -> u8 {
        if self.pos < self.code.len() {
            self.pos += 1;
        }
        self.next()
    }
    fn step_while<F: Fn(u8) -> bool>(&mut self, f: F) {
        while self.next() != 0 && f(self.next()) {
            self.pos += 1;
        }
    }
    fn as_slice(&self) -> &[u8] {
        &self.code[self.pos..]
    }
    fn take(&mut self) -> &'a [u8] {
        let result = &self.code[..self.pos];
        self.code = &self.code[self.pos..];
        self.pos = 0;
        result
    }
    fn reset(&mut self) {
        self.pos = 0;
    }
}

pub fn transform(code: &[u8]) -> Vec<u8> {
    let tt = parse(code);

    let tt = apply_renames(tt);
    let tt = apply_transform_to_load(tt);

    serialize(&tt, b' ')
}

fn apply_renames(tt: TokenTree) -> TokenTree {
    fn inner(mut tt: TokenTree, outer_renames: &HashMap<Vec<u8>, Vec<u8>>) -> TokenTree {
        let mut renames = outer_renames.clone();
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

        let mut new_tt = vec![];

        for token in tt {
            match token {
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
                        new_tt.push(token);
                    }
                }
                TreeToken::Token { .. } => {
                    new_tt.push(token);
                }
                TreeToken::SubTree(sub_tt) => {
                    new_tt.push(TreeToken::SubTree(inner(sub_tt, &renames)));
                }
            }
        }

        new_tt
    }
    inner(tt, &HashMap::new())
}

fn apply_transform_to_load(tt: TokenTree) -> TokenTree {
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
                    let body = serialize(&sub_tt[4..(sub_tt.len() - 1)], b' ');
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
                        && last_token_text.ends_with(b"0")) =>
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

fn parse(code: &[u8]) -> TokenTree {
    fn parse_subtree(tokens: &mut TokenTree, code: &mut Input) {
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
    let mut code = Input::new(code);
    parse_subtree(&mut tokens, &mut code);

    tokens
}

#[derive(Debug)]
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
    EOF,
    Other,
}

fn next_token<'a>(code: &mut Input<'a>) -> (TokenType, &'a [u8]) {
    code.step_while(|c| c.is_ascii_whitespace());
    code.take();

    let c = code.next();
    code.step();

    if c == 0 {
        return (TokenType::EOF, code.take());
    }

    if c == b'-' && code.next() == b'-' {
        code.step_while(|c| c != b'\n' && c != b'\r');
        return (TokenType::Comment, code.take());
    }

    if c == b'_' || c.is_ascii_alphabetic() {
        code.step_while(|c| c == b'_' || c.is_ascii_alphanumeric());
        return (TokenType::Identifier, code.take());
    }

    if c.is_ascii_digit() || c == b'.' {
        if c == b'0' && code.next().to_ascii_lowercase() == b'x' {
            code.step();
        }
        code.step_while(|c| c == b'.' || c.is_ascii_hexdigit());
        return (TokenType::Number, code.take());
    }

    if c == b'"' || c == b'\'' {
        loop {
            if code.next() == c {
                break;
            }
            if code.next() == b'\\' {
                code.step();
            }
            code.step();
        }
        code.step();
    }

    if c == b'[' {
        let mut count = 0;
        while code.next() == b'=' {
            count += 1;
            code.step();
        }
        if code.next() == b'[' {
            let mut end_marker = vec![b']'];
            for _ in 0..count {
                end_marker.push(b'=');
            }
            end_marker.push(b']');
            while code.next() != 0 && !code.as_slice().starts_with(&end_marker) {
                code.step();
            }
            for _ in 0..(count + 2) {
                code.step();
            }
        } else {
            code.reset();
            code.step();
        }
    }

    (TokenType::Other, code.take())
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn multiline_strings() {
        let mut input = Input::new(b"[==[foo[=[bar]=]baz]==]...");
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"[==[foo[=[bar]=]baz]==]");
    }

    #[test]
    fn strings() {
        let mut input = Input::new(b"\"test\\\"a\\\"\"  'foo\\''");
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"\"test\\\"a\\\"\"");
        let (tpe, bytes) = next_token(&mut input);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"'foo\\''");
    }
}
