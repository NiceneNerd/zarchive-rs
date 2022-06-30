[![crates.io](https://img.shields.io/crates/v/zarchive)](https://crates.io/crates/zarchive)
[![api](https://img.shields.io/badge/api-rustdoc-558b2f)](https://docs.rs/zarchive)
[![license](https://img.shields.io/badge/license-GPL-blue)](https://spdx.org/licenses/GPL-3.0-or-later.html)
[![build](https://img.shields.io/github/workflow/status/NiceneNerd/zarchive-rs/Build%20and%20test)](https://github.com/NiceneNerd/zarchive-rs/actions/workflows/push.yml)

Simple Rust bindings to [ZArchive](https://github.com/Exzap/ZArchive).
## Overview
ZArchive is yet another file archive format. Think of zip, tar, 7z, etc. but with the
requirement of allowing random-access reads and supporting compression.

## Features / Specifications
- Supports random-access reads within stored files
- Uses zstd compression (64KiB blocks)
- Scales reasonably well up to multiple terabytes with millions of files
- The theoretical size limit per-file is 2^48-1 (256 Terabyte)
- The encoding for paths within the archive is Windows-1252 (case-insensitive)
- Contains a SHA256 hash of the whole archive for integrity checks
- Endian-independent. The format always uses big-endian internally
- Stateless file and directory iterator handles which don't require memory allocation
  (not entirely true of the Rust bindings)

## Rust Bindings
The `zarchive` crate provides Rust bindings to the C++ library. The API is intentionally
limited. While most of the reader API is implemented, only a basic archive packing function
is exposed for writing, due to complex safety considerations.
The Rust bindings add some slight overhead to the reader's directory iteration API,
but hopefully with a sufficient benefit of convenience.

## Example - Pack and extract an archive
```ignore
use zarchive::{pack, extract};
pack("/path/to/stuff/to/pack", "/path/to/archive.zar")?;
extract("/path/to/archive.zar", "/path/to/extract")?;
```

## Limitations
- Not designed for adding, removing or modifying files after the archive has been created
- 
## No-seek creation
When creating new archives only byte append operations are used. No file seeking is
necessary. This makes it possible to create archives on storage which is write-once.
It also simplifies streaming ZArchive creation over network.

## UTF8 paths
UTF8 for file and folder paths is theoretically supported as paths are just binary
blobs. But the case-insensitive comparison only applies to latin letters (a-z).
The Rust bindings use the primitive `&str` type, which means that all paths passed
through this API are UTF8.

## Wii U specifics
Originally this format was created to store Wii U games dumps. These use the file
extension .wua (Wii U Archive) but are otherwise regular ZArchive files. To allow
multiple Wii U titles to be stored inside a single archive, each title must be placed
in a subfolder following the naming scheme: 16-digit titleId followed by \_v and then
the version as decimal. For example: `0005000e10102000_v32`

## License
The `zarchive` crate is licensed under the GPL 3+. The original ZArchive library is
licensed under [MIT No Attribution](https://github.com/Exzap/ZArchive/blob/master/LICENSE),
with the exception of [sha_256.c](/src/sha_256.c) and [sha_256.h](/src/sha_256.h)
which are public domain, see:
[https://github.com/amosnier/sha-2]( https://github.com/amosnier/sha-2).