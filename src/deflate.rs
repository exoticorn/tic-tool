use super::cp437;
use anyhow::Result;

pub fn analyze(data: &[u8]) -> Analysis {
    let mut bitstream = Bitstream::new(data);
    let mut data = AnalysisData {
        unpacked: vec![],
        literal_index: vec![],
        cost: vec![],
    };

    let mut blocks: Vec<BlockAnalysis> = vec![];

    let mut is_final = false;
    while !is_final {
        is_final = bitstream.get_bit() == 1;
        let block_type = bitstream.get_bits(2);
        let header_item = bitstream.take_item();
        match block_type {
            1 => {
                let mut huff_lit_length = HuffmanBuilder::new();
                huff_lit_length.add_codes(0..=143, 8);
                huff_lit_length.add_codes(144..=255, 9);
                huff_lit_length.add_codes(256..=279, 7);
                huff_lit_length.add_codes(280..=287, 8);

                let mut huff_distance = HuffmanBuilder::new();
                huff_distance.add_codes(0..=31, 5);

                let lz_items = decode_block(
                    &mut bitstream,
                    &mut data,
                    huff_lit_length.build(),
                    huff_distance.build(),
                );

                blocks.push(BlockAnalysis {
                    header_item,
                    block_type: BlockType::StaticHuffman,
                    lz: lz_items,
                });
            }
            2 => {
                let hlit = bitstream.get_bits(5) as usize;
                let hdist = bitstream.get_bits(5) as usize;
                let hclen = bitstream.get_bits(4) as usize;
                let huff_header_item = bitstream.take_item();
                let mut huff_header = HuffmanBuilder::new();
                let mut huff_header_lengths = vec![];
                for &code in &[
                    16u32, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ][..hclen + 4]
                {
                    let length = bitstream.get_bits(3);
                    huff_header.add_code(code, length);
                    huff_header_lengths.push((code, length, bitstream.take_item()));
                }
                let huff_header = huff_header.build();
                let mut huff_lengths = vec![0u32; hlit + 257 + hdist + 1];
                let mut pos = 0;
                let mut huff_header_codes = vec![];
                while pos < huff_lengths.len() {
                    let code = huff_header.read(&mut bitstream);
                    let huff_item = bitstream.take_item();
                    match code {
                        16 => {
                            let count = bitstream.get_bits(2) + 3;
                            huff_header_codes.push(HuffmanHeaderCode::Repeat {
                                huff_item,
                                count_item: bitstream.take_item(),
                                count,
                            });
                            for _ in 0..count {
                                huff_lengths[pos] = huff_lengths[pos - 1];
                                pos += 1;
                            }
                        }
                        17 => {
                            let count = bitstream.get_bits(3) + 3;
                            huff_header_codes.push(HuffmanHeaderCode::Skip {
                                huff_item,
                                count_item: bitstream.take_item(),
                                count,
                            });
                            for _ in 0..count {
                                huff_lengths[pos] = 0;
                                pos += 1;
                            }
                        }
                        18 => {
                            let count = bitstream.get_bits(7) + 11;
                            huff_header_codes.push(HuffmanHeaderCode::Skip {
                                huff_item,
                                count_item: bitstream.take_item(),
                                count,
                            });
                            for _ in 0..count {
                                huff_lengths[pos] = 0;
                                pos += 1;
                            }
                        }
                        num_bits => {
                            huff_header_codes.push(HuffmanHeaderCode::Length {
                                huff_item,
                                length: num_bits,
                            });
                            huff_lengths[pos] = num_bits;
                            pos += 1;
                        }
                    }
                }

                let mut huff_lit_length = HuffmanBuilder::new();
                for code in 0..hlit + 257 {
                    huff_lit_length.add_code(code as u32, huff_lengths[code]);
                }
                let mut huff_distance = HuffmanBuilder::new();
                for code in 0..hdist + 1 {
                    huff_distance.add_code(code as u32, huff_lengths[hlit + 257 + code]);
                }

                let lz_items = decode_block(
                    &mut bitstream,
                    &mut data,
                    huff_lit_length.build(),
                    huff_distance.build(),
                );

                blocks.push(BlockAnalysis {
                    header_item,
                    block_type: BlockType::DynamicHuffman {
                        huff_header_item,
                        hlit,
                        hdist,
                        hclen,
                        huff_header_lengths,
                        huff_header_codes,
                    },
                    lz: lz_items,
                });
            }
            _ => panic!("Block type {} not implemented yet", block_type),
        }
    }

    let mut ref_count = vec![0usize; data.unpacked.len()];
    for &index in &data.literal_index {
        if index != usize::MAX {
            ref_count[index] += 1;
        }
    }
    let mut shifted = vec![];
    for (&index, &cost) in data.literal_index.iter().zip(data.cost.iter()) {
        if index != usize::MAX {
            let delta = (data.cost[index] - cost) / (ref_count[index] + 1) as f32;
            shifted.push(delta);
            shifted[index] -= delta;
        } else {
            shifted.push(0.);
        }
    }
    for (cost, delta) in data.cost.iter_mut().zip(shifted.into_iter()) {
        *cost += delta;
    }

    Analysis { data, blocks }
}

pub struct Analysis {
    data: AnalysisData,
    blocks: Vec<BlockAnalysis>,
}

impl Analysis {
    pub fn disassemble(&self) {
        let mut pos = 0;
        for block in &self.blocks {
            match block.block_type {
                BlockType::StaticHuffman => {
                    disass_line(&[&block.header_item], format!("static huffman block"));
                }
                BlockType::DynamicHuffman {
                    ref huff_header_item,
                    hlit,
                    hdist,
                    hclen,
                    ref huff_header_lengths,
                    ref huff_header_codes,
                } => {
                    disass_line(&[&block.header_item], format!("dynamic huffman block"));
                    disass_line(
                        &[huff_header_item],
                        format!("hlit: {}, hdist: {}, hclen: {}", hlit, hdist, hclen),
                    );
                    for &(code, length, ref item) in huff_header_lengths {
                        disass_line(
                            &[item],
                            format!("huffman encoding - code {:-2}, {} bits", code, length),
                        );
                    }
                    let mut c = 0u32;
                    for code in huff_header_codes {
                        match *code {
                            HuffmanHeaderCode::Length {
                                ref huff_item,
                                length,
                            } => {
                                let name = if c < 256 {
                                    format!("'{}'", cp437::MAPPING[c as usize])
                                } else if c == 256 {
                                    "EOB".to_string()
                                } else if c < hlit as u32 + 257 {
                                    format!("length({})", c - 257)
                                } else {
                                    format!("offset({})", c - 257 - hlit as u32)
                                };
                                disass_line(&[huff_item], format!("{} - {} bits", name, length));
                                c += 1;
                            }
                            HuffmanHeaderCode::Repeat {
                                ref huff_item,
                                ref count_item,
                                count,
                            } => {
                                disass_line(&[huff_item, count_item], format!("repeat {}x", count));
                                c += count;
                            }
                            HuffmanHeaderCode::Skip {
                                ref huff_item,
                                ref count_item,
                                count,
                            } => {
                                disass_line(&[huff_item, count_item], format!("skip {}", count));
                                c += count;
                            }
                        }
                    }
                }
            }

            for lz_item in &block.lz {
                match *lz_item {
                    LzItem::EndOfBlock { ref item } => {
                        disass_line(&[item], format!("end of block"))
                    }
                    LzItem::Literal { ref item, byte } => {
                        disass_line(&[item], format!("lit '{}'", cp437::MAPPING[byte as usize]));
                        pos += 1;
                    }
                    LzItem::Match {
                        length,
                        offset,
                        ref length_base,
                        ref length_ext,
                        ref offset_base,
                        ref offset_ext,
                    } => {
                        let mut copy_string = String::new();
                        for i in 0..length {
                            copy_string.push(
                                cp437::MAPPING
                                    [self.data.unpacked[(pos - offset + i) as usize] as usize],
                            );
                        }
                        pos += length;
                        disass_line(
                            &[length_base, length_ext, offset_base, offset_ext],
                            format!("mtc {} @ {}: '{}'", length, offset, copy_string),
                        );
                    }
                }
            }
        }
    }

    pub fn print_heatmap(&self) -> Result<()> {
        use crossterm::{
            style::{Attribute, Color},
            terminal,
        };
        let colors = [
            (Color::DarkCyan, Color::White),
            (Color::DarkGreen, Color::White),
            (Color::Black, Color::White),
            (Color::DarkBlue, Color::White),
            (Color::DarkMagenta, Color::White),
            (Color::DarkYellow, Color::White),
            (Color::Red, Color::Black),
            (Color::White, Color::Black),
        ];
        let term_width = terminal::size()?.0.min(120);
        let mut pos = 1;
        print!(" ");
        for ((&byte, &cost), &ref_index) in self
            .data
            .unpacked
            .iter()
            .zip(self.data.cost.iter())
            .zip(self.data.literal_index.iter())
        {
            if pos + 1 == term_width {
                print!("\n ");
                pos = 1;
            }
            let color = colors[(cost.round() as usize).max(1).min(8) - 1];
            print!(
                "{}",
                crossterm::style::style(cp437::MAPPING[byte as usize])
                    .with(color.1)
                    .on(color.0)
                    .attribute(if ref_index == usize::MAX {
                        Attribute::NoUnderline
                    } else {
                        Attribute::Underlined
                    })
            );
            pos += 1;
        }
        println!("\n");
        print!("Legend: :");
        for (i, &(b, f)) in colors.iter().enumerate() {
            print!("{}", crossterm::style::style(i + 1).with(f).on(b));
        }
        println!(" bits");
        Ok(())
    }
}

fn disass_line(items: &[&BitstreamItem], text: String) {
    let pos = items[0].pos;
    print!("{:-4x}.{}", pos >> 3, pos & 7);
    let mut padding = 24;
    for item in items {
        if item.length + 1 > padding {
            print!("\n      ");
            padding = 24;
        }
        print!(" ");
        for i in (0..item.length).rev() {
            print!("{}", (item.bits >> i) & 1);
        }
        padding -= item.length + 1;
    }
    for _ in 0..padding + 2 {
        print!(" ");
    }
    println!("{}", text);
}

struct AnalysisData {
    unpacked: Vec<u8>,
    literal_index: Vec<usize>,
    cost: Vec<f32>,
}

struct BlockAnalysis {
    block_type: BlockType,
    header_item: BitstreamItem,
    lz: Vec<LzItem>,
}

enum BlockType {
    //    Uncompressed,
    StaticHuffman,
    DynamicHuffman {
        huff_header_item: BitstreamItem,
        hlit: usize,
        hdist: usize,
        hclen: usize,
        huff_header_lengths: Vec<(u32, u32, BitstreamItem)>,
        huff_header_codes: Vec<HuffmanHeaderCode>,
    },
}

enum LzItem {
    Literal {
        byte: u8,
        item: BitstreamItem,
    },
    Match {
        length: u32,
        offset: u32,
        length_base: BitstreamItem,
        length_ext: BitstreamItem,
        offset_base: BitstreamItem,
        offset_ext: BitstreamItem,
    },
    EndOfBlock {
        item: BitstreamItem,
    },
}

enum HuffmanHeaderCode {
    Length {
        huff_item: BitstreamItem,
        length: u32,
    },
    Repeat {
        huff_item: BitstreamItem,
        count_item: BitstreamItem,
        count: u32,
    },
    Skip {
        huff_item: BitstreamItem,
        count_item: BitstreamItem,
        count: u32,
    },
}

fn decode_block(
    bitstream: &mut Bitstream,
    data: &mut AnalysisData,
    huff_lit_length: Huffman,
    huff_distance: Huffman,
) -> Vec<LzItem> {
    let mut lz_items = vec![];
    loop {
        let lit_length = huff_lit_length.read(bitstream);
        let lit_length_item = bitstream.take_item();
        if lit_length == 256 {
            lz_items.push(LzItem::EndOfBlock {
                item: lit_length_item,
            });
            return lz_items;
        }

        if lit_length < 256 {
            data.cost.push(lit_length_item.length as f32);
            lz_items.push(LzItem::Literal {
                item: lit_length_item,
                byte: lit_length as u8,
            });
            data.unpacked.push(lit_length as u8);
            data.literal_index.push(usize::MAX);
        } else {
            let (extra_bits, base_length) = [
                (0, 3),
                (0, 4),
                (0, 5),
                (0, 6),
                (0, 7),
                (0, 8),
                (0, 9),
                (0, 10),
                (1, 11),
                (1, 13),
                (1, 15),
                (1, 17),
                (2, 19),
                (2, 23),
                (2, 27),
                (2, 31),
                (3, 35),
                (3, 43),
                (3, 51),
                (3, 59),
                (4, 67),
                (4, 83),
                (4, 99),
                (5, 131),
                (5, 163),
                (5, 195),
                (5, 227),
                (0, 258),
            ][lit_length as usize - 257];
            let length = base_length + bitstream.get_bits(extra_bits);
            let length_ext = bitstream.take_item();
            let offset_index = huff_distance.read(bitstream);
            let offset_base = bitstream.take_item();
            let (extra_bits, base_distance) = [
                (0, 1),
                (0, 2),
                (0, 3),
                (0, 4),
                (1, 5),
                (1, 7),
                (2, 9),
                (2, 13),
                (3, 17),
                (3, 25),
                (4, 33),
                (4, 49),
                (5, 65),
                (5, 97),
                (6, 129),
                (6, 193),
                (7, 257),
                (7, 385),
                (8, 513),
                (8, 769),
                (9, 1025),
                (9, 1537),
                (10, 2049),
                (10, 3073),
                (11, 4097),
                (11, 6145),
                (12, 8193),
                (12, 12289),
                (13, 16385),
                (13, 24577),
            ][offset_index as usize];
            let distance = base_distance + bitstream.get_bits(extra_bits);
            let offset_ext = bitstream.take_item();
            let cost = (lit_length_item.length
                + length_ext.length
                + offset_base.length
                + offset_ext.length) as f32
                / length as f32;
            lz_items.push(LzItem::Match {
                length,
                offset: distance,
                length_base: lit_length_item,
                length_ext,
                offset_base,
                offset_ext,
            });
            let copy_base = data.unpacked.len() - distance as usize;
            for i in 0..length {
                let lit_index = data.literal_index[copy_base + i as usize];
                data.literal_index.push(if lit_index == usize::MAX {
                    copy_base + i as usize
                } else {
                    lit_index
                });
                data.unpacked.push(data.unpacked[copy_base + i as usize]);
                data.cost.push(cost);
            }
        }
    }
}

struct HuffmanBuilder {
    codes: Vec<(u32, u32)>,
}

impl HuffmanBuilder {
    fn new() -> HuffmanBuilder {
        HuffmanBuilder { codes: vec![] }
    }

    fn add_code(&mut self, code: u32, num_bits: u32) {
        if num_bits > 0 {
            self.codes.push((code, num_bits));
        }
    }

    fn add_codes<I: IntoIterator<Item = u32>>(&mut self, codes: I, num_bits: u32) {
        if num_bits > 0 {
            for code in codes.into_iter() {
                self.codes.push((code, num_bits));
            }
        }
    }

    fn build(mut self) -> Huffman {
        self.codes
            .sort_unstable_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        Huffman { codes: self.codes }
    }
}

struct Huffman {
    codes: Vec<(u32, u32)>,
}

impl Huffman {
    fn read(&self, bitstream: &mut Bitstream) -> u32 {
        let mut code = 0;
        let mut num_bits = 0;
        for &(value, length) in &self.codes {
            while num_bits < length {
                code = (code << 1) | bitstream.get_bit();
                num_bits += 1;
            }
            if code == 0 {
                return value;
            }
            code -= 1;
        }
        panic!("No value found for huffman code")
    }
}

struct Bitstream<'a> {
    data: &'a [u8],
    pos: usize,
    item_start: usize,
}

struct BitstreamItem {
    pos: usize,
    length: usize,
    bits: u32,
}

impl<'a> Bitstream<'a> {
    fn new(data: &'a [u8]) -> Bitstream<'a> {
        Bitstream {
            data,
            pos: 0,
            item_start: 0,
        }
    }

    fn get_bit(&mut self) -> u32 {
        let bit = (self.data[self.pos >> 3] >> (self.pos & 7)) as u32 & 1;
        self.pos += 1;
        bit
    }

    fn get_bits(&mut self, num_bits: u32) -> u32 {
        let mut value = 0;
        for i in 0..num_bits {
            value |= self.get_bit() << i;
        }
        value
    }

    fn take_item(&mut self) -> BitstreamItem {
        let length = self.pos - self.item_start;
        assert!(length <= 32);
        let pos = self.item_start;
        self.pos = pos;
        let bits = self.get_bits(length as u32);
        self.item_start = self.pos;
        BitstreamItem { pos, length, bits }
    }
}
