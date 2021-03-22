mod cp437;
mod deflate;
mod lua;
mod tic_file;

use anyhow::{anyhow, bail, Result};
use clap::Clap;
use flate2::write::ZlibEncoder;
use std::{
    cmp,
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    process::exit,
};
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
    #[clap(short, long, about = "Automatically apply rename suggestions")]
    auto_rename: bool,
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
        if self.no_transform && self.auto_rename {
            eprintln!("Both --no-transform and --auto-rename specified. Auto renaming needs transforms to be active.");
            exit(1);
        }

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
            let mut program = lua::Program::parse(&code);
            code = program.serialize(b' ');
            let source_renames = program.renames.clone();

            fn print_renames(renames: lua::Renaming) {
                let mut renames: Vec<(Vec<u8>, Vec<u8>)> = renames.into_iter().collect();
                renames.sort();
                for (src, dst) in renames {
                    println!(
                        "-- rename {}->{}",
                        std::str::from_utf8(&src).unwrap(),
                        std::str::from_utf8(&dst).unwrap()
                    );
                }
                println!();
            }

            let mut analysis = deflate::analyze(&zopfli(&code));

            if self.auto_rename {
                let mut rename: lua::Renaming = source_renames;
                let mut best_rename = rename.clone();
                let mut best_size = analysis.total_size();
                let mut best_code = code;
                let mut seen_renames: HashSet<lua::Renaming> = HashSet::new();
                seen_renames.insert(rename.clone());

                loop {
                    let new_rename = compute_rename_suggestions(&program, &analysis);
                    rename = merge_renames(&rename, &new_rename);
                    if !seen_renames.insert(rename.clone()) {
                        break;
                    }
                    program.apply_renames(&new_rename);
                    let new_code = program.serialize(b' ');
                    analysis = deflate::analyze(&zopfli(&new_code));
                    let size = analysis.total_size();
                    if size < best_size {
                        best_rename = rename.clone();
                        best_size = size;
                        best_code = new_code;
                    }
                }

                code = best_code;

                println!("Best auto renames found:\n");
                print_renames(best_rename);
            } else {
                println!("Suggested renames:\n");
                print_renames(merge_renames(
                    &source_renames,
                    &compute_rename_suggestions(&program, &analysis),
                ));
            }
        }

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

fn zopfli(code: &[u8]) -> Vec<u8> {
    let mut compressed = vec![];
    zopfli_rs::compress(
        &zopfli_rs::Options::default(),
        &zopfli_rs::Format::Deflate,
        code,
        &mut compressed,
    )
    .unwrap();
    compressed
}

fn compute_rename_suggestions(
    program: &lua::Program,
    analysis: &deflate::Analysis,
) -> lua::Renaming {
    let candidates = program.get_rename_candidates();
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
    // print!("renameable ids:");
    // for &(ref id, count, _) in &renameable_ids {
    //     print!("  {}: {}", std::str::from_utf8(id).unwrap(), count.ceil());
    // }
    // println!();

    let mut candidate_ids: HashMap<Vec<u8>, (f32, usize)> = HashMap::new();
    for &offset in &candidates.candidate_chars {
        if analysis.literal_index[offset] == usize::MAX {
            let c = analysis.unpacked[offset];
            if !lua::is_valid_ident_start(c) {
                dbg!(std::str::from_utf8(&analysis.unpacked[offset - 10..offset]).unwrap());
                dbg!(std::str::from_utf8(&analysis.unpacked[offset..offset + 10]).unwrap());
            }
            candidate_ids
                .entry(vec![analysis.unpacked[offset]])
                .or_insert_with(|| (0., offset))
                .0 += if analysis.block_type[offset] == 2 {
                1.
            } else {
                0.1
            };
        }
    }
    let id_count = renameable_ids.len();

    let mut candidate_ids: Vec<(Vec<u8>, f32, usize)> = candidate_ids
        .into_iter()
        .map(|(c, (count, offset))| (c, count, offset))
        .collect();
    fn white_space_efficiency(c: u8) -> u8 {
        match c | 32 {
            b'a'..=b'f' => 0,
            b'p' | b'x' => 1,
            _ => 2,
        }
    }
    candidate_ids.sort_unstable_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(cmp::Ordering::Less)
            .then(white_space_efficiency(b.0[0]).cmp(&white_space_efficiency(a.0[0])))
            .then(a.2.cmp(&b.2))
    });
    // print!("candidate ids:");
    // for &(ref id, count, _) in &candidate_ids {
    //     print!("  {}: {}", std::str::from_utf8(id).unwrap(), count.ceil());
    // }
    // println!();

    let mut candidate_ids: Vec<Vec<u8>> = candidate_ids.into_iter().map(|(id, ..)| id).collect();

    if id_count > candidate_ids.len() {
        let mut used_ids = candidates.fixed;
        used_ids.extend(candidate_ids.iter().cloned());
        for &c in b"_ghijklmnoqrstuvwyzpxabcdefGHIJKLMNOQRSTUVWYZPXABCDEF" {
            let id = vec![c];
            if used_ids.insert(id.clone()) {
                candidate_ids.push(id);
            }
        }
        let mut pos = 0usize;
        while id_count > candidate_ids.len() {
            let d = ((pos as f32 * 2. + 0.75).sqrt() - 0.5).floor() as usize;
            let x = pos - d * (d + 1) / 2;
            let y = d - x;
            let mut id = candidate_ids[y].clone();
            id.extend_from_slice(&candidate_ids[x]);
            if used_ids.insert(id.clone()) {
                candidate_ids.push(id);
            }
            pos += 1;
        }
    }

    renameable_ids
        .into_iter()
        .map(|(id, ..)| id)
        .zip(candidate_ids.into_iter().take(id_count))
        .collect()
}

fn merge_renames(a: &lua::Renaming, b: &lua::Renaming) -> lua::Renaming {
    let reverse: BTreeMap<&Vec<u8>, &Vec<u8>> = a.iter().map(|(src, dst)| (dst, src)).collect();
    let mut a = a.clone();
    a.extend(b.iter().map(|(src, dst)| {
        if let Some(&prev_src) = reverse.get(&src) {
            (prev_src.clone(), dst.clone())
        } else {
            (src.clone(), dst.clone())
        }
    }));
    a
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

    if code.len() <= 1024 {
        println!("Heatmap:\n");
        analysis.print_heatmap().unwrap();
        println!();
    }

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
