mod tic_file;

use anyhow::Result;
use clap::Clap;
use std::{io::prelude::*, path::PathBuf};

#[derive(Clap)]
#[clap(version = "0.1.0", author = "Dennis Ranke <dennis.ranke@gmail.com>")]
struct Opts {
    input: PathBuf,
    output: PathBuf,
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    let chunks = tic_file::load(opts.input)?;

    let mut out_chunks = vec![];

    let mut new_palette_default: Option<tic_file::Chunk> = None;
    for chunk in chunks {
        match chunk.type_ {
            0x11 => new_palette_default = Some(chunk),
            0x05 => out_chunks.push(compress_code(chunk.data)),
            0x10 => {
                let mut unpacked = vec![];
                libflate::deflate::Decoder::new(&chunk.data[2..]).read_to_end(&mut unpacked)?;
                out_chunks.push(compress_code(unpacked));
            }
            _ => (),
        }
    }
    out_chunks.extend(new_palette_default.into_iter());

    tic_file::save(opts.output, &out_chunks)?;

    Ok(())
}

fn compress_code(code: Vec<u8>) -> tic_file::Chunk {
    let code = strip_whitespace(&code);
    println!("Uncompressed size: {:5} bytes", code.len());
    let mut data = vec![];
    zopfli::compress(
        &zopfli::Options::default(),
        &zopfli::Format::Zlib,
        &code,
        &mut data,
    )
    .unwrap();
    data.truncate(data.len() - 4);
    println!("  Compressed size: {:5} bytes", data.len());
    tic_file::Chunk {
        type_: 0x10,
        bank: 0,
        data,
    }
}

fn strip_whitespace(code: &[u8]) -> Vec<u8> {
    let mut stripped = vec![];

    #[derive(Eq, PartialEq)]
    enum State {
        Normal,
        Minus,
        Comment,
        Identifier,
        WsAfterIdentifier,
        Number,
        WsAfterNumber,
        String(u8),
        StringEscape(u8),
    }

    let mut state = State::Normal;
    for &c in code {
        match state {
            State::Normal => match c {
                b' ' | b'\n' | b'\r' | b'\t' => (),
                b'_' | b'A'..=b'Z' | b'a'..=b'z' => {
                    state = State::Identifier;
                    stripped.push(c);
                }
                b'0'..=b'9' | b'.' => {
                    state = State::Number;
                    stripped.push(c);
                }
                b'-' => state = State::Minus,
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                _ => stripped.push(c),
            },
            State::Minus => match c {
                b'-' => state = State::Comment,
                b'_' | b'A'..=b'Z' | b'a'..=b'z' => {
                    state = State::Identifier;
                    stripped.push(b'-');
                    stripped.push(c);
                }
                b'0'..=b'9' | b'.' => {
                    state = State::Number;
                    stripped.push(b'-');
                    stripped.push(c);
                }
                b' ' | b'\n' | b'\r' | b'\t' => {
                    state = State::Normal;
                    stripped.push(b'-');
                }
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                _ => {
                    state = State::Normal;
                    stripped.push(b'-');
                    stripped.push(c);
                }
            },
            State::Comment => match c {
                b'\n' | b'\r' => state = State::Normal,
                _ => (),
            },
            State::Identifier => match c {
                b' ' | b'\n' | b'\r' | b'\t' => state = State::WsAfterIdentifier,
                b'_' | b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => stripped.push(c),
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                b'-' => state = State::Minus,
                _ => {
                    state = State::Normal;
                    stripped.push(c);
                }
            },
            State::Number => match c {
                b' ' | b'\n' | b'\r' | b'\t' => state = State::WsAfterNumber,
                b'0'..=b'9' | b'A'..=b'F' | b'a'..=b'f' | b'X' | b'x' | b'.' => stripped.push(c),
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                b'-' => state = State::Minus,
                _ => {
                    state = State::Normal;
                    stripped.push(c);
                }
            },
            State::WsAfterIdentifier => match c {
                b' ' | b'\n' | b'\r' | b'\t' => (),
                b'_' | b'A'..=b'Z' | b'a'..=b'z' => {
                    state = State::Identifier;
                    stripped.push(b' ');
                    stripped.push(c);
                }
                b'0'..=b'9' | b'.' => {
                    state = State::Number;
                    stripped.push(b' ');
                    stripped.push(c);
                }
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                b'-' => state = State::Minus,
                _ => {
                    state = State::Normal;
                    stripped.push(c)
                }
            },
            State::WsAfterNumber => match c {
                b' ' | b'\n' | b'\r' | b'\t' => (),
                b'A'..=b'F' | b'a'..=b'f' => {
                    state = State::Identifier;
                    stripped.push(b' ');
                    stripped.push(c);
                }
                b'_' | b'G'..=b'Z' | b'g'..=b'z' => {
                    state = State::Identifier;
                    stripped.push(c);
                }
                b'0'..=b'9' | b'.' => {
                    state = State::Number;
                    stripped.push(b' ');
                    stripped.push(c);
                }
                b'\'' | b'"' => {
                    state = State::String(c);
                    stripped.push(c);
                }
                b'-' => state = State::Minus,
                _ => {
                    state = State::Normal;
                    stripped.push(c)
                }
            },
            State::String(q) => {
                stripped.push(c);
                if c == q {
                    state = State::Normal;
                } else if c == b'\\' {
                    state = State::StringEscape(q);
                }
            }
            State::StringEscape(q) => {
                stripped.push(c);
                state = State::String(q);
            }
        }
    }

    stripped
}
