mod tic_file;
mod lua;
mod cp437;

use anyhow::{anyhow, bail, Result};
use clap::Clap;
use std::{collections::HashMap, fs::File, io::prelude::*, sync::mpsc, time::Duration};

use std::path::PathBuf;

#[derive(Clap)]
#[clap(version = "0.1.1", author = "Dennis Ranke <dennis.ranke@gmail.com>")]
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
}

fn main() -> Result<()> {
    let opts = Opts::parse();

    match opts.cmd {
        SubCommand::Pack(pack) => pack.exec()?,
        SubCommand::Extract(cmd) => cmd.exec()?,
        SubCommand::Empty(cmd) => cmd.exec()?,
    }

    Ok(())
}

#[derive(Clap)]
struct CmdPack {
    #[clap(short = 'k', long, about = "Don't transform (whitespace/directives) as lua src")]
    no_transform: bool,
    #[clap(short, long, about = "Strip chunks except for code and new palette")]
    strip: bool,
    #[clap(short, long, about = "Force new palette")]
    new_palette: bool,
    #[clap(short, long, about = "Watch for the source file to be updated")]
    watch: bool,
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
            code = lua::transform(&code);
        }

        if self.new_palette {
            new_palette_default = Some(tic_file::Chunk {
                type_: 0x11,
                bank: 0,
                data: vec![],
            });
        }

        out_chunks.push(compress_code(code));
        out_chunks.extend(new_palette_default.into_iter());

        tic_file::save(&self.output, &out_chunks)?;

        Ok(())
    }
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

fn compress_code(code: Vec<u8>) -> tic_file::Chunk {
    print_char_distribution(&code);

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

fn print_char_distribution(code: &[u8]) {
    use crossterm::{style::{Color, Colors, SetColors, ResetColor}, ExecutableCommand};
    use std::io::stdout;
    let mut counts: HashMap<u8, usize> = HashMap::new();
    for &c in code {
        *counts.entry(c).or_default() += 1;
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
    let mut stdout = stdout();
    let colors = [Color::DarkRed, Color::DarkYellow, Color::Black, Color::DarkGreen, Color::DarkBlue, Color::DarkMagenta];
    let blocks = ['\u{2588}', '\u{2593}', '\u{2592}', '\u{2591}', ' '];
    for &(_, count) in &counts {
        let heat = (count as f32 * counts.len() as f32 / code.len() as f32).ln() / 1.5f32.ln();
        let heat = (0.5 - heat / 4.).max(0.).min(1.) * colors.len() as f32;
        let index = (heat as usize).min(colors.len() - 2);
        stdout.execute(SetColors(Colors::new(colors[index], colors[index+1]))).unwrap();
        let frac = heat - index as f32;
        let block_index = (frac * blocks.len() as f32 - 0.5).max(0.).min(blocks.len() as f32 - 1.) as usize;
        print!("{}", blocks[block_index]);
    }
    stdout.execute(ResetColor).unwrap();
    println!();

    println!();
}
