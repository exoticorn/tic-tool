use super::cp437;

pub fn analyze(data: &[u8]) {
    let mut bitstream = Bitstream::new(data);
    let mut unpacked = vec![];

    let mut is_final = false;
    while !is_final {
        is_final = bitstream.get_bit() == 1;
        let block_type = bitstream.get_bits(2);
        dbg!(is_final, block_type);
        match block_type {
            1 => {
                let mut huff_lit_length = HuffmanBuilder::new();
                huff_lit_length.add_codes(0..=143, 8);
                huff_lit_length.add_codes(144..=255, 9);
                huff_lit_length.add_codes(256..=279, 7);
                huff_lit_length.add_codes(280..=287, 8);

                let mut huff_distance = HuffmanBuilder::new();
                huff_distance.add_codes(0..=31, 5);

                decode_block(
                    &mut bitstream,
                    &mut unpacked,
                    huff_lit_length.build(),
                    huff_distance.build(),
                );
            }
            2 => {
                let hlit = bitstream.get_bits(5) as usize;
                let hdist = bitstream.get_bits(5) as usize;
                let hclen = bitstream.get_bits(4) as usize;
                let mut huff_header = HuffmanBuilder::new();
                for &code in &[
                    16u32, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ][..hclen + 4]
                {
                    huff_header.add_code(code, bitstream.get_bits(3));
                }
                let huff_header = huff_header.build();
                let mut huff_lengths = vec![0u32; hlit + 257 + hdist + 1];
                let mut pos = 0;
                while pos < huff_lengths.len() {
                    match huff_header.read(&mut bitstream) {
                        16 => {
                            for _ in 0..bitstream.get_bits(2) + 3 {
                                huff_lengths[pos] = huff_lengths[pos - 1];
                                pos += 1;
                            }
                        }
                        17 => {
                            for _ in 0..bitstream.get_bits(3) + 3 {
                                huff_lengths[pos] = 0;
                                pos += 1;
                            }
                        }
                        18 => {
                            for _ in 0..bitstream.get_bits(7) + 11 {
                                huff_lengths[pos] = 0;
                                pos += 1;
                            }
                        }
                        num_bits => {
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

                decode_block(
                    &mut bitstream,
                    &mut unpacked,
                    huff_lit_length.build(),
                    huff_distance.build(),
                );
            }
            _ => panic!("Block type {} not implemented yet", block_type),
        }
    }
}

fn decode_block(
    bitstream: &mut Bitstream,
    unpacked: &mut Vec<u8>,
    huff_lit_length: Huffman,
    huff_distance: Huffman,
) {
    loop {
        let lit_length = huff_lit_length.read(bitstream);
        if lit_length == 256 {
            return;
        }

        if lit_length < 256 {
            println!(
                "Literal: {} '{}'",
                lit_length,
                cp437::MAPPING[lit_length as usize]
            );
            unpacked.push(lit_length as u8);
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
            ][huff_distance.read(bitstream) as usize];
            let distance = base_distance + bitstream.get_bits(extra_bits);
            let copy_base = unpacked.len() - distance as usize;
            print!("copy {} from offset {}: '", length, distance);
            for i in 0..length {
                let byte = unpacked[copy_base + i as usize];
                unpacked.push(byte);
                print!("{}", cp437::MAPPING[byte as usize]);
            }
            println!("'");
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
    bits: u32,
    left: u32,
}

impl<'a> Bitstream<'a> {
    fn new(data: &'a [u8]) -> Bitstream<'a> {
        Bitstream {
            data,
            bits: 0,
            left: 0,
        }
    }

    fn get_bit(&mut self) -> u32 {
        self.ensure_bits(1);
        let bit = self.bits & 1;
        self.bits >>= 1;
        self.left -= 1;
        bit
    }

    fn get_bits(&mut self, num_bits: u32) -> u32 {
        self.ensure_bits(num_bits);
        let value = self.bits & ((1 << num_bits) - 1);
        self.bits >>= num_bits;
        self.left -= num_bits;
        value
    }

    fn ensure_bits(&mut self, num_bits: u32) {
        while self.left < num_bits {
            self.bits |= (self.data[0] as u32) << self.left;
            self.data = &self.data[1..];
            self.left += 8;
        }
    }
}
