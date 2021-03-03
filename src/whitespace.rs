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

pub fn strip_whitespace(code: &[u8]) -> Vec<u8> {
    lua::strip_whitespace(code)
    // TODO: add javacsript support
}

mod lua {
    use super::*;

    pub fn strip_whitespace(code: &[u8]) -> Vec<u8> {
        let mut code = Input::new(code);
        let mut stripped = vec![];

        let mut last_token_type = TokenType::Other;

        loop {
            let (token_type, token_bytes) = next_token(&mut code);
            if token_type == TokenType::EOF {
                break;
            }

            match last_token_type {
                TokenType::Identifier if token_bytes[0] == b'_' || token_bytes[0].is_ascii_alphanumeric() => {
                    stripped.push(b' ');
                }
                TokenType::Number if token_bytes[0] == b'.' || token_bytes[0].is_ascii_hexdigit() => {
                    stripped.push(b' ');
                }
                _ => ()
            }
            stripped.extend_from_slice(token_bytes);
            last_token_type = token_type;
        }

        stripped
    }

    #[derive(PartialEq, Eq, Debug)]
    enum TokenType {
        Identifier,
        Number,
        EOF,
        Other,
    }

    fn next_token<'a>(code: &mut Input<'a>) -> (TokenType, &'a [u8]) {
        loop {
            if code.as_slice().starts_with(b"--") {
                code.step_while(|c| c != b'\n' && c != b'\r');
            }
            if !code.next().is_ascii_whitespace() {
                code.take();
                break;
            }
            code.step();
        }

        let c = code.next();
        code.step();

        if c == 0 {
            return (TokenType::EOF, code.take());
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
        use super::super::Input;
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
}
