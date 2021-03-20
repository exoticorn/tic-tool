mod cp437;
mod deflate;
mod lua;
mod tic_file;

use anyhow::{anyhow, bail, Result};
use clap::Clap;
use flate2::write::ZlibEncoder;
use std::{cmp, path::PathBuf};
use std::{collections::HashMap, fs::File, io::prelude::*, sync::mpsc, time::Duration};

#[derive(Clap)]
#[clap(version = "0.2.0", author = "Dennis Ranke <dennis.ranke@gmail.com>")]
struct Opts {
    #[clap(subcommand)]
    pub cmd: SubCommand,
}

#[derive(Clap)]
enum SubCommand {
    #[clap(about = "Create a .tic file with compressed code chunk")]
    Pack(CmdPack),
    #[clap(about = "Extract code chunk of a .tic file")]
    Extract(CmdExtract),
    #[clap(about = "Create an empty .tic file")]
    Empty(CmdEmpty),
    #[clap(about = "Print out detailed information about a .tic file")]
    Analyze(CmdAnalyze),
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    match opts.cmd {
        SubCommand::Pack(pack) => pack.exec()?,
        SubCommand::Extract(cmd) => cmd.exec()?,
        SubCommand::Empty(cmd) => cmd.exec()?,
        SubCommand::Analyze(cmd) => cmd.exec()?,
    }

    Ok(())
}

#[derive(Clap)]
struct CmdPack {
    #[clap(
        short = 'k',
        long,
        about = "Don't transform (whitespace/directives) as lua src"
    )]
    no_transform: bool,
    #[clap(short, long, about = "Strip chunks except for code and new palette")]
    strip: bool,
    #[clap(short, long, about = "Force new palette")]
    new_palette: bool,
    #[clap(short, long, about = "Watch for the source file to be updated")]
    watch: bool,
    #[clap(
        short,
        long,
        default_value = "15",
        about = "Number of zopfli iterations (default 15)"
    )]
    iterations: u32,
    #[clap(about = "Either a .tic file or source code")]
    input: PathBuf,
    output: PathBuf,
}

impl CmdPack {
    fn exec(self) -> Result<()> {
        self.run()?;
        if self.watch {
            use notify::{DebouncedEvent, RecursiveMode, Watcher};
            let (tx, rx) = mpsc::channel();
            let mut watcher = notify::watcher(tx, Duration::from_millis(20))?;

            watcher.watch(&self.input, RecursiveMode::NonRecursive)?;
            loop {
                if let DebouncedEvent::Write(_) = rx.recv()? {
                    println!();
                    self.run()?;
                }
            }
        }

        Ok(())
    }

    fn run(&self) -> Result<()> {
        let mut out_chunks = vec![];

        let mut new_palette_default: Option<tic_file::Chunk> = None;
        let mut code: Option<Vec<u8>> = None;

        if self.input.extension().map_or(false, |ext| ext == "tic") {
            let chunks = tic_file::load(&self.input)?;
            for chunk in chunks {
                match chunk.type_ {
                    0x11 => new_palette_default = Some(chunk),
                    0x05 => code = Some(chunk.data),
                    0x10 => {
                        let mut unpacked = vec![];
                        libflate::deflate::Decoder::new(&chunk.data[2..])
                            .read_to_end(&mut unpacked)?;
                        code = Some(unpacked);
                    }
                    _ if self.strip => (),
                    _ => out_chunks.push(chunk),
                }
            }
        } else {
            let mut buffer = vec![];
            File::open(&self.input)?.read_to_end(&mut buffer)?;
            code = Some(buffer);
        }

        let mut code = code.ok_or_else(|| anyhow!("No code chunk found"))?;
        if !self.no_transform {
            code = lua::Program::parse(&code).serialize(b' ');
        }

        compute_rename_suggestions(&code);

        if self.new_palette {
            new_palette_default = Some(tic_file::Chunk {
                type_: 0x11,
                bank: 0,
                data: vec![],
            });
        }

        out_chunks.push(compress_code(code, self.iterations as i32));
        out_chunks.extend(new_palette_default.into_iter());

        tic_file::save(&self.output, &out_chunks)?;

        Ok(())
    }
}

fn compute_rename_suggestions(code: &[u8]) {
    let program = lua::Program::parse(code);

    let candidates = program.get_rename_candidates();

    let mut compressed = vec![];
    zopfli_rs::compress(
        &zopfli_rs::Options::default(),
        &zopfli_rs::Format::Deflate,
        &code,
        &mut compressed,
    )
    .unwrap();

    let analysis = deflate::analyze(&compressed);
    let analysis = analysis.data();

    let mut renameable_count: HashMap<Vec<u8>, (f32, usize)> = HashMap::new();
    for (id, offsets) in &candidates.renameable {
        let count = renameable_count
            .entry(id.clone())
            .or_insert_with(|| (0., offsets[0]));
        for &offset in offsets {
            for o in offset..offset + id.len() {
                if analysis.literal_index[o] == usize::MAX {
                    count.0 += if analysis.block_type[o] == 2 { 1. } else { 0.1 };
                }
            }
        }
    }

    let mut renameable_ids: Vec<(Vec<u8>, f32, usize)> = renameable_count
        .into_iter()
        .map(|(id, (count, offset))| {
            let count = count / id.len() as f32;
            (id, count, offset)
        })
        .collect();
    renameable_ids.sort_unstable_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(cmp::Ordering::Less)
            .then(a.2.cmp(&b.2))
    });
    print!("renameable ids:");
    for &(ref id, count, _) in &renameable_ids {
        print!("  {}: {}", std::str::from_utf8(id).unwrap(), count.ceil());
    }
    println!();

    let mut candidate_ids: HashMap<u8, (f32, usize)> = HashMap::new();
    for &offset in &candidates.candidate_chars {
        if analysis.literal_index[offset] == usize::MAX {
            candidate_ids
                .entry(code[offset])
                .or_insert_with(|| (0., offset))
                .0 += if analysis.block_type[offset] == 2 {
                1.
            } else {
                0.1
            };
        }
    }
    let mut candidate_ids: Vec<(u8, f32, usize)> = candidate_ids
        .into_iter()
        .map(|(c, (count, offset))| (c, count, offset))
        .collect();
    candidate_ids.sort_unstable_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(cmp::Ordering::Less)
            .then(a.2.cmp(&b.2))
    });
    print!("candidate ids:");
    for &(c, count, _) in &candidate_ids {
        print!("  {}: {}", c as char, count.ceil());
    }
    println!();
}

#[derive(Clap)]
struct CmdExtract {
    input: PathBuf,
    output: PathBuf,
}

impl CmdExtract {
    fn exec(self) -> Result<()> {
        let chunks = tic_file::load(self.input)?;
        fn find_code(chunks: Vec<tic_file::Chunk>) -> Result<Vec<u8>> {
            for chunk in chunks {
                match chunk.type_ {
                    0x05 => return Ok(chunk.data),
                    0x10 => {
                        let mut unpacked = vec![];
                        libflate::deflate::Decoder::new(&chunk.data[2..])
                            .read_to_end(&mut unpacked)?;
                        return Ok(unpacked);
                    }
                    _ => (),
                }
            }
            bail!("No code chunk found");
        }
        let code = find_code(chunks)?;
        File::create(self.output)?.write_all(&code)?;
        Ok(())
    }
}

#[derive(Clap)]
struct CmdEmpty {
    #[clap(short, long, about = "Use new palette")]
    new_palette: bool,
    output: PathBuf,
}

impl CmdEmpty {
    fn exec(self) -> Result<()> {
        let mut chunks = vec![tic_file::Chunk {
            type_: 0x05,
            bank: 0,
            data: vec![],
        }];
        if self.new_palette {
            chunks.push(tic_file::Chunk {
                type_: 0x11,
                bank: 0,
                data: vec![],
            });
        }
        tic_file::save(self.output, &chunks)?;
        Ok(())
    }
}

fn compress_code(code: Vec<u8>, iterations: i32) -> tic_file::Chunk {
    let mut data = vec![];
    zopfli_rs::compress(
        &zopfli_rs::Options {
            iterations,
            ..Default::default()
        },
        &zopfli_rs::Format::Zlib,
        &code,
        &mut data,
    )
    .unwrap();
    data.truncate(data.len() - 4);
    let zopfli_size = data.len();

    let mut zlib_encoder = ZlibEncoder::new(vec![], flate2::Compression::best());
    zlib_encoder.write_all(&code).unwrap();
    let mut dataz = zlib_encoder.finish().unwrap();
    dataz.truncate(dataz.len() - 4);
    let zlib_size = dataz.len();

    if dataz.len() < data.len() {
        data = dataz;
    }

    let analysis = deflate::analyze(&data[2..]);

    print_char_distribution(analysis.data());

    println!("Heatmap:\n");
    analysis.print_heatmap().unwrap();
    println!();

    analysis.print_sizes();
    println!();

    println!("         Uncompressed size: {:5} bytes", code.len());
    println!("  Compressed size (Zopfli): {:5} bytes", zopfli_size);
    println!("    Compressed size (zlib): {:5} bytes", zlib_size);

    if code.len() <= data.len() {
        tic_file::Chunk {
            type_: 0x05,
            bank: 0,
            data: code,
        }
    } else {
        tic_file::Chunk {
            type_: 0x10,
            bank: 0,
            data,
        }
    }
}

fn print_char_distribution(data: &deflate::AnalysisData) {
    use crossterm::style::Color;
    let mut counts: HashMap<u8, usize> = HashMap::new();
    let mut total = 0;
    for (&c, &lit_index) in data.unpacked.iter().zip(data.literal_index.iter()) {
        if lit_index == usize::MAX {
            *counts.entry(c).or_default() += 1;
            total += 1;
        }
    }
    let mut counts: Vec<(u8, usize)> = counts.into_iter().collect();
    counts.sort_by_key(|&(_, count)| count);
    counts.reverse();
    println!("Number of unique chars: {}", counts.len());
    print!(" ");
    for &(c, _) in &counts {
        print!("{}", cp437::MAPPING[c as usize]);
    }
    println!();
    print!(" ");
    let colors = [
        Color::DarkRed,
        Color::DarkYellow,
        Color::Black,
        Color::DarkGreen,
        Color::DarkBlue,
        Color::DarkMagenta,
    ];
    let blocks = ['\u{2588}', '\u{2593}', '\u{2592}', '\u{2591}', ' '];
    for &(_, count) in &counts {
        let heat = (count as f32 * counts.len() as f32 / total as f32).ln() / 1.5f32.ln();
        let heat = (0.5 - heat / 4.).max(0.).min(1.) * colors.len() as f32;
        let index = (heat as usize).min(colors.len() - 2);
        let frac = heat - index as f32;
        let block_index = (frac * blocks.len() as f32 - 0.5)
            .max(0.)
            .min(blocks.len() as f32 - 1.) as usize;
        print!(
            "{}",
            crossterm::style::style(blocks[block_index])
                .with(colors[index])
                .on(colors[index + 1])
        );
    }
    println!();

    println!();
}

#[derive(Clap)]
struct CmdAnalyze {
    input: PathBuf,
}

impl CmdAnalyze {
    fn exec(self) -> Result<()> {
        let chunks = tic_file::load(self.input)?;

        for chunk in chunks {
            println!("Chunk {:02x} - len {}", chunk.type_, chunk.data.len());

            match chunk.type_ {
                0x10 => {
                    let analysis = deflate::analyze(&chunk.data[2..]);
                    analysis.disassemble();
                    analysis.print_heatmap()?;
                }
                _ => (),
            }
        }

        Ok(())
    }
}
