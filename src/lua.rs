use lazy_static::lazy_static;
use regex::bytes::Regex;
use std::collections::{BTreeMap, HashMap, HashSet};

pub type Renaming = BTreeMap<Vec<u8>, Vec<u8>>;

pub struct Program {
    tt: TokenTree,
    pub renames: Renaming,
}
#[derive(Debug)]
pub struct RenameCandidates {
    pub renameable: HashMap<Vec<u8>, Vec<usize>>,
    pub fixed: HashSet<Vec<u8>>,
    pub candidate_chars: Vec<usize>,
}

impl Program {
    pub fn parse(code: &[u8]) -> Program {
        let tt = parse(code);
        let (tt, renames) = find_renames(tt);
        let tt = apply_renames(&tt, &renames);
        let tt = apply_transform_to_load(tt);
        Program { tt, renames }
    }

    pub fn apply_renames(&mut self, renames: &Renaming) {
        self.tt = apply_renames(&self.tt, renames);
    }

    pub fn serialize(&mut self, ws: u8) -> Vec<u8> {
        serialize(&mut self.tt, ws)
    }

    pub fn get_rename_candidates(&self) -> RenameCandidates {
        let mut candidates = RenameCandidates {
            renameable: HashMap::new(),
            fixed: HashSet::new(),
            candidate_chars: Vec::new(),
        };

        let renameable_ids = find_renamable_identifiers(&self.tt);

        fn inner(
            candidates: &mut RenameCandidates,
            tt: &TokenTree,
            renameable_ids: &HashSet<Vec<u8>>,
            delim_stack: DelimStack,
        ) {
            for token in tt {
                match *token {
                    TreeToken::Token {
                        type_: TokenType::Comment,
                        ..
                    } => (),
                    TreeToken::Token {
                        type_: TokenType::Identifier,
                        offset,
                        ref text,
                    } => {
                        if renameable_ids.contains(text) {
                            candidates
                                .renameable
                                .entry(text.clone())
                                .or_default()
                                .push(offset);
                        } else {
                            candidates.fixed.insert(text.clone());
                            for i in 0..text.len() {
                                if is_valid_ident_start(text[i]) {
                                    candidates.candidate_chars.push(offset + i);
                                }
                            }
                        }
                    }
                    TreeToken::Token {
                        mut offset,
                        ref text,
                        ..
                    } => {
                        for &c in text {
                            offset += delim_stack.encode_length(c) - 1;
                            if is_valid_ident_start(c) {
                                candidates.candidate_chars.push(offset);
                            }
                            offset += 1;
                        }
                    }
                    TreeToken::SubTree(ref sub_tt) => {
                        inner(candidates, sub_tt, renameable_ids, delim_stack.clone())
                    }
                    TreeToken::CodeString {
                        tt: ref sub_tt,
                        delim,
                    } => inner(candidates, sub_tt, renameable_ids, delim_stack.push(delim)),
                }
            }
        }

        inner(
            &mut candidates,
            &self.tt,
            &renameable_ids,
            DelimStack::empty(),
        );

        candidates
    }
}

pub fn is_valid_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn find_renames(mut tt: TokenTree) -> (TokenTree, Renaming) {
    let mut renames = BTreeMap::new();
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^--\s*rename\s*(\w+)\s*->\s*(\w+)\s*$").unwrap();
    }
    tt.retain(|tok| {
        if let &TreeToken::Token {
            type_: TokenType::Comment,
            ref text,
            ..
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

fn apply_renames(tt: &TokenTree, renames: &Renaming) -> TokenTree {
    let mut new_tt = vec![];

    for token in tt {
        match *token {
            TreeToken::Token {
                type_: TokenType::Identifier,
                ref text,
                ..
            } => {
                if let Some(new_name) = renames.get(text) {
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        offset: 0,
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
            TreeToken::CodeString {
                tt: ref sub_tt,
                delim,
            } => {
                new_tt.push(TreeToken::CodeString {
                    tt: apply_renames(sub_tt, renames),
                    delim,
                });
            }
        }
    }

    new_tt
}

fn apply_transform_to_load(tt: TokenTree) -> TokenTree {
    let mut new_tt = vec![];

    let mut transform_next = false;
    for token in tt {
        match token {
            TreeToken::Token {
                type_: TokenType::Comment,
                ref text,
                ..
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
                    let body = sub_tt[4..(sub_tt.len() - 1)].to_vec();
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        offset: 0,
                        text: name.to_vec(),
                    });
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Other,
                        offset: 0,
                        text: b"=".to_vec(),
                    });
                    new_tt.push(TreeToken::Token {
                        type_: TokenType::Identifier,
                        offset: 0,
                        text: b"load".to_vec(),
                    });
                    new_tt.push(TreeToken::CodeString {
                        tt: body,
                        delim: b'"',
                    });
                    transform_next = false;
                } else {
                    new_tt.push(token);
                }
            }
            TreeToken::CodeString { .. } => new_tt.push(token),
        }
    }

    new_tt
}

fn find_renamable_identifiers(tt: &TokenTree) -> HashSet<Vec<u8>> {
    fn inner(idents: &mut HashSet<Vec<u8>>, tt: &TokenTree) {
        for (index, token) in tt.iter().enumerate() {
            match (token, tt.get(index + 1)) {
                (
                    TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: ref id_name,
                        ..
                    },
                    Some(&TreeToken::Token {
                        type_: TokenType::Other,
                        ref text,
                        ..
                    }),
                ) if text == b"=" => match id_name.as_slice() {
                    b"TIC" | b"SCN" | b"OVR" => (),
                    _ => {
                        idents.insert(id_name.clone());
                    }
                },
                (
                    TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: ref fn_text,
                        ..
                    },
                    Some(&TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: ref id_name,
                        ..
                    }),
                ) if fn_text == b"function" => match id_name.as_slice() {
                    b"TIC" | b"SCN" | b"OVR" => (),
                    _ => {
                        idents.insert(id_name.clone());
                    }
                },
                (TreeToken::SubTree(ref sub_tt), _) => inner(idents, sub_tt),
                (TreeToken::CodeString { tt: ref sub_tt, .. }, _) => inner(idents, sub_tt),
                _ => (),
            }
        }
    }
    let mut idents = HashSet::new();
    inner(&mut idents, tt);
    idents
}

#[derive(Clone)]
struct DelimStack(Vec<u8>);

impl DelimStack {
    fn empty() -> DelimStack {
        DelimStack(vec![])
    }

    fn push(&self, delim: u8) -> DelimStack {
        let mut v = vec![delim];
        v.extend_from_slice(&self.0);
        DelimStack(v)
    }

    fn encode(&self, dst: &mut Vec<u8>, c: u8) {
        fn inner(dst: &mut Vec<u8>, c: u8, stack: &[u8]) {
            if !stack.is_empty() {
                if c == b'\\' || c == stack[0] {
                    inner(dst, b'\\', &stack[1..]);
                }
                inner(dst, c, &stack[1..]);
            } else {
                dst.push(c);
            }
        }
        inner(dst, c, &self.0);
    }

    fn encode_length(&self, c: u8) -> usize {
        fn inner(c: u8, stack: &[u8]) -> usize {
            let mut length = 0;
            if !stack.is_empty() {
                if c == b'\\' || c == stack[0] {
                    length += inner(b'\\', &stack[1..]);
                }
                length += inner(c, &stack[1..]);
            } else {
                length = 1;
            }
            length
        }
        inner(c, &self.0)
    }
}

fn serialize(tt: &mut [TreeToken], ws: u8) -> Vec<u8> {
    struct LastToken {
        type_: TokenType,
        text: Vec<u8>,
    }

    fn inner(
        tt: &mut [TreeToken],
        last_token: &mut LastToken,
        code: &mut Vec<u8>,
        ws: u8,
        delim_stack: DelimStack,
    ) {
        for token in tt {
            match *token {
                TreeToken::Token {
                    type_,
                    ref mut offset,
                    ref text,
                } => {
                    if type_ == TokenType::Comment {
                        continue;
                    }

                    match last_token.type_ {
                        TokenType::Identifier
                            if text[0] == b'_' || text[0].is_ascii_alphanumeric() =>
                        {
                            code.push(ws);
                        }
                        TokenType::Number
                            if text[0] == b'.'
                                || text[0].is_ascii_hexdigit()
                                || (text[0].to_ascii_lowercase() == b'x'
                                    && (last_token.text == b"0" || last_token.text == b".0")) =>
                        {
                            code.push(ws);
                        }
                        TokenType::HexNumber
                            if text[0] == b'.'
                                || text[0].is_ascii_hexdigit()
                                || text[0].to_ascii_lowercase() == b'p' =>
                        {
                            code.push(ws);
                        }
                        _ => (),
                    }
                    *offset = code.len();
                    for &c in text {
                        delim_stack.encode(code, c);
                    }
                    last_token.type_ = type_;
                    last_token.text = text.clone();
                }

                TreeToken::SubTree(ref mut sub_tt) => {
                    inner(sub_tt, last_token, code, ws, delim_stack.clone());
                }
                TreeToken::CodeString {
                    tt: ref mut sub_tt,
                    delim,
                } => {
                    delim_stack.encode(code, delim);
                    last_token.type_ = TokenType::Other;
                    inner(sub_tt, last_token, code, ws, delim_stack.push(delim));
                    delim_stack.encode(code, delim);
                }
            }
        }
    }

    let mut code = vec![];
    let mut last_token = LastToken {
        type_: TokenType::Other,
        text: vec![],
    };
    inner(tt, &mut last_token, &mut code, ws, DelimStack::empty());
    code
}

fn parse(code: &[u8]) -> TokenTree {
    fn parse_subtree(tokens: &mut TokenTree, code: &[u8], offset: &mut usize) {
        loop {
            let (token_type, token_text, token_start) = next_token(code, offset);
            if token_type == TokenType::EOF {
                return;
            }
            if token_type == TokenType::Identifier {
                if token_text == b"function" {
                    let mut sub_tokens = vec![];
                    sub_tokens.push(TreeToken::Token {
                        type_: token_type,
                        offset: token_start,
                        text: token_text.to_vec(),
                    });
                    parse_subtree(&mut sub_tokens, code, offset);
                    tokens.push(TreeToken::SubTree(sub_tokens));
                    continue;
                }
                if token_text == b"do" || token_text == b"if" {
                    tokens.push(TreeToken::Token {
                        type_: token_type,
                        offset: token_start,
                        text: token_text.to_vec(),
                    });
                    parse_subtree(tokens, code, offset);
                    continue;
                }
            }
            tokens.push(TreeToken::Token {
                type_: token_type,
                offset: token_start,
                text: token_text.to_vec(),
            });
            if token_type == TokenType::Identifier && token_text == b"end" {
                return;
            }
        }
    }
    let mut tokens = vec![];
    let mut offset = 0;
    parse_subtree(&mut tokens, code, &mut offset);

    fn parse_load_functions(tt: TokenTree) -> TokenTree {
        lazy_static! {
            static ref CODE_STRING_COMMENT: Regex = Regex::new(r"\A--\s*code\s+string").unwrap();
        }
        let mut new_tt = vec![];

        let mut index = 0;
        while index < tt.len() {
            let token = &tt[index];
            index += 1;
            fn make_code_string(text: &[u8], offset: usize) -> TreeToken {
                let mut code = vec![];
                let mut offset_map: HashMap<usize, usize> = HashMap::new();
                let mut pos = 1;
                while pos + 1 < text.len() {
                    offset_map.insert(code.len(), offset + pos);
                    code.push(match text[pos] {
                        b'\\' => {
                            pos += 1;
                            match text[pos] {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                b'\\' => b'\\',
                                o => o,
                            }
                        }
                        o => o,
                    });
                    pos += 1;
                }
                let mut sub_tt = parse(&code);
                fn remap(tt: &mut TokenTree, offset_map: &HashMap<usize, usize>) {
                    for token in tt {
                        match token {
                            TreeToken::Token { ref mut offset, .. } => {
                                *offset = *offset_map.get(offset).unwrap()
                            }
                            TreeToken::SubTree(ref mut sub_tt) => remap(sub_tt, offset_map),
                            TreeToken::CodeString {
                                tt: ref mut sub_tt, ..
                            } => remap(sub_tt, offset_map),
                        }
                    }
                }
                remap(&mut sub_tt, &offset_map);
                TreeToken::CodeString {
                    tt: parse_load_functions(sub_tt),
                    delim: text[0],
                }
            }
            match (token, tt.get(index)) {
                (
                    &TreeToken::Token {
                        type_: TokenType::Identifier,
                        text: ref fn_name,
                        ..
                    },
                    Some(&TreeToken::Token {
                        type_: TokenType::String,
                        offset,
                        ref text,
                    }),
                ) if fn_name == b"load" => {
                    new_tt.push(token.clone());
                    new_tt.push(make_code_string(text, offset));
                    index += 1;
                }
                (
                    &TreeToken::Token {
                        type_: TokenType::Comment,
                        text: ref comment,
                        ..
                    },
                    Some(&TreeToken::Token {
                        type_: TokenType::String,
                        offset,
                        ref text,
                    }),
                ) if CODE_STRING_COMMENT.is_match(comment) => {
                    new_tt.push(make_code_string(text, offset));
                    index += 1;
                }
                _ => new_tt.push(token.clone()),
            }
        }

        new_tt
    }

    parse_load_functions(tokens)
}

#[derive(Debug, Clone)]
enum TreeToken {
    Token {
        type_: TokenType,
        offset: usize,
        text: Vec<u8>,
    },
    SubTree(TokenTree),
    CodeString {
        tt: TokenTree,
        delim: u8,
    },
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
    String,
    EOF,
    Other,
}

fn next_token<'a>(code: &'a [u8], offset: &mut usize) -> (TokenType, &'a [u8], usize) {
    lazy_static! {
        static ref WHITE_SPACE: Regex = Regex::new(r"\A\s+").unwrap();
        static ref LONG_BRACKET_COMMENT: Regex = Regex::new(r"\A--\[=*\[").unwrap();
        static ref COMMENT: Regex = Regex::new(r"\A--.*").unwrap();
        static ref IDENTIFIER: Regex = Regex::new(r"\A[_a-zA-Z][_a-zA-Z0-9]*").unwrap();
        static ref NUMBER: Regex = Regex::new(r"\A(\d+(\.\d*)?|\.\d+)([eE]-?\d+)?").unwrap();
        static ref HEXNUMBER: Regex =
            Regex::new(r"\A0[xX][[:xdigit:]]*(\.[[:xdigit:]]*)?([pP]-?\d+)?").unwrap();
        static ref LONG_BRACKET: Regex = Regex::new(r"\A\[=*\[").unwrap();
        static ref COMPOUND_OPERATOR: Regex = Regex::new(r"\A(==|~=|<=|>=)").unwrap();
    }

    if let Some(m) = WHITE_SPACE.find(&code[*offset..]) {
        *offset += m.end();
    }

    let start_offset = *offset;

    let code = &code[*offset..];

    if let Some(m) = LONG_BRACKET_COMMENT.find(code) {
        let len = find_long_bracket_end(code, m.end() - 4);
        let string = &code[..len];
        *offset += len;
        return (TokenType::Comment, string, start_offset);
    }

    if let Some(m) = COMMENT.find(code) {
        *offset += m.end();
        return (TokenType::Comment, m.as_bytes(), start_offset);
    }

    if let Some(m) = IDENTIFIER.find(code) {
        *offset += m.end();
        return (TokenType::Identifier, m.as_bytes(), start_offset);
    }

    if let Some(m) = HEXNUMBER.find(code) {
        *offset += m.end();
        return (TokenType::HexNumber, m.as_bytes(), start_offset);
    }

    if let Some(m) = NUMBER.find(code) {
        *offset += m.end();
        return (TokenType::Number, m.as_bytes(), start_offset);
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
            *offset += pos;
            return (TokenType::String, string, start_offset);
        }
    }

    if let Some(m) = LONG_BRACKET.find(code) {
        let len = find_long_bracket_end(code, m.end() - 2);
        let string = &code[..len];
        *offset += len;
        return (TokenType::Other, string, start_offset);
    }

    if let Some(m) = COMPOUND_OPERATOR.find(code) {
        *offset += m.end();
        return (TokenType::Other, m.as_bytes(), start_offset);
    }

    if code.len() > 0 {
        let tok = &code[..1];
        *offset += 1;
        return (TokenType::Other, tok, start_offset);
    }

    return (TokenType::EOF, b"", start_offset);
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
        let input: &[u8] = b"[==[foo[=[bar]=]baz]==]...";
        let mut offset = 0;
        let (tpe, bytes, _) = next_token(&input, &mut offset);
        assert_eq!(tpe, TokenType::Other);
        assert_eq!(bytes, b"[==[foo[=[bar]=]baz]==]");
    }

    #[test]
    fn strings() {
        let input: &[u8] = b"\"test\\\"a\\\"\"  'foo\\''";
        let mut offset = 0;
        let (tpe, bytes, _) = next_token(&input, &mut offset);
        assert_eq!(tpe, TokenType::String);
        assert_eq!(bytes, b"\"test\\\"a\\\"\"");
        let (tpe, bytes, _) = next_token(&input, &mut offset);
        assert_eq!(tpe, TokenType::String);
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

    #[test]
    fn rename_inside_load() {
        assert_eq!(transform(b"--rename a->b\nA=load\"a=2\""), b"A=load\"b=2\"");
    }
}
