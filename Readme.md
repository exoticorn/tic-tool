# tic-tool

`tic-tool` is asimple tool to process TIC-80 files (`.tic`). It's most important use is to compress the source code of a `.tic` file.

## Features

* Shrink the size of the code in a `.tic` file using compression, whitespace removal and some simple transforms.
* Extract the code from a `.tic` file.
* Create an empty `.tic` file with just an empty code chunk and (optionally) a `0x11` chunk (new default palette)

## Usage

```
USAGE:
    tic-tool <SUBCOMMAND>

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    empty      Create an empty .tic file
    extract    Extract code chunk of a .tic file
    help       Prints this message or the help of the given subcommand(s)
    pack       Create a .tic file with compressed code chunk
```

Use `tick-tool help <subcommand>` for help on the commands themselves.

## Compressing code

```
    tic-tool pack [FLAGS] <input> <output>

ARGS:
    <input>     Either a .tic file or source code
    <output>    

FLAGS:
    -h, --help            Prints help information
    -n, --new-palette     Force new palette
    -k, --no-transform    Don't transform (whitespace/directives) as lua src
    -s, --strip           Strip chunks except for code and new palette
    -V, --version         Prints version information
    -w, --watch           Watch for the source file to be updated
```

`tic-tool pack` reads either a `.tic` file, or just a source file (for example `.lua`) and outputs a `.tic` file with the source code compressed using the zopfli compression library and optionally shrunk by removing all unnecessary whitespace.

When using a `.tic` file as input, `tic-tool` will keep all chunks other than code exactly as they are by default, except for making sure a `0x11` chunk if existing is placed at the very end so that it can be truncated, saving 3 bytes.

`-s/--strip` will instead remove all chunks except for code and `0x11`. This oviously will do nothing when the input is a source file.

`-n/--new-palette` will add a `0x11` chunk if it's not already there.

`-k/--no-transform` will disable whitespace/comment removal and the code transforms detailed below. Since both of these assume the code is in lua you might need to use this flag for other languages.

`-w/--watch` will keep the tool running, waiting for the input file to change and the reprocess it. This time you can check the compressed size any time you save the file.

### Transforms

There are currently two types of transforms you can use by placing directives in comments in your source code:

`-- rename a->b` will rename all occurancies of identifier `a` to `b`.

`-- transform to load` will transform the next function taking no parameters from it's normal form
```
function NAME()
  ...
end
```
to the shorter form
```
NAME=load"..."
```

### Output

During packing, the tool will output the following information:

* The number of unique characters/bytes used in the source code
* The characters sorted by descending count (with a colored bar below it showing the rough distribution)
* The size of the code pre-compression/post-compression
* The total size of the resulting `.tic` file

### Example usage:

With `metropolis.lua`:
```
-- rename x -> d
-- rename y -> a
t=9

-- transform to load
function TIC()
    t = t + .5
    for o = 0, 3e4 do
        y = o / t + t
        x = o % 240 - t / 3
        poke4(o, (((math.atan(x, y) * t / 4 + t) // 1 | (8 ^ 8 / (x * x + y * y)) // 9) & 6) - 3)
    end
end
```
(`metropolis '80` by superogue/Marquee Design)

```
> tic-tool pack metropolis.lua metropolis.tic
Number of unique chars: 40
 toa/()d= +4,3*en".-8902f^&|pl6h5ImT%r1Ck

Uncompressed size:   118 bytes
  Compressed size:   110 bytes
       Total size:   114 bytes
```