use anyhow::Result;
use bytes::{Buf, BufMut, BytesMut};
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;

pub struct Chunk {
    pub type_: u8,
    pub bank: u8,
    pub data: Vec<u8>,
}

pub fn load<P: AsRef<Path>>(filename: P) -> Result<Vec<Chunk>> {
    let mut file = vec![];
    File::open(filename)?.read_to_end(&mut file)?;
    let mut file = &file[..];
    let mut chunks = vec![];

    while file.remaining() > 1 {
        let (type_, bank) = {
            let v = file.get_u8();
            (v & 31, v >> 5)
        };
        let length = if file.remaining() >= 2 {
            file.get_u16_le() as usize
        } else {
            0
        };
        let mut data = vec![];
        if file.remaining() >= 1 + length {
            file.advance(1);
            data.resize(length, 0);
            file.copy_to_slice(&mut data);
        } else {
            file.advance(file.remaining());
        }
        chunks.push(Chunk { type_, bank, data });
    }

    Ok(chunks)
}

pub fn save<P: AsRef<Path>>(filename: P, chunks: &[Chunk]) -> Result<()> {
    let mut file = BytesMut::new();
    for (i, chunk) in chunks.iter().enumerate() {
        file.put_u8(chunk.type_ | (chunk.bank << 5));
        if i + 1 < chunks.len() || !chunk.data.is_empty() || chunk.type_ != 0x11  {
            file.put_u16_le(chunk.data.len() as u16);
        }
        if i + 1 < chunks.len() || !chunk.data.is_empty() {
            file.put_u8(0);
            file.put(&chunk.data[..]);
        }
    }
    println!("                Total size: {:5} bytes", file.len());
    File::create(filename)?.write_all(&file[..])?;
    Ok(())
}