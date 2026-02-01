use std::{
    borrow::Cow,
    env,
    ffi::{OsStr, OsString, os_str},
    fmt::{self, Display, Formatter},
    fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, anyhow};
use object::{Architecture, Object as _, ObjectSymbol as _, read::archive::ArchiveFile};

macro_rules! write_bytes {
    ($dst:expr, $($bytes:expr),* $(,)?) => {
        (|| {
            use std::io::{IoSlice, Error, ErrorKind};
            let mut bufs = [
                $(
                    IoSlice::new($bytes.as_ref()),
                )*
                IoSlice::new(b"\n"),
            ];
            // TODO(https://github.com/rust-lang/rust/issues/70436): use `write_all_vectored` when stable.
            let mut bufs = &mut bufs[..];
            IoSlice::advance_slices(&mut bufs, 0);
            while !bufs.is_empty() {
                match $dst.write_vectored(bufs) {
                    Ok(0) => {
                        return Err(Error::new(ErrorKind::WriteZero, "failed to write whole buffer"));
                    }
                    Ok(n) => IoSlice::advance_slices(&mut bufs, n),
                    Err(ref e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })().map_err(Into::<anyhow::Error>::into)
    };
}

enum Cxxstdlibs<'a> {
    EnvVar(OsString),
    Single(&'static [u8]),
    Multiple(&'a [&'static [u8]]),
}

impl Cxxstdlibs<'_> {
    /// Detects which standard C++ library to link.
    fn new(stdout: &mut io::StdoutLock<'_>) -> anyhow::Result<Self> {
        const CXXSTDLIB: &str = "CXXSTDLIB";
        writeln!(stdout, "cargo:rerun-if-env-changed={CXXSTDLIB}")?;
        Ok(match env::var_os(CXXSTDLIB) {
            Some(cxxstdlib) => Self::EnvVar(cxxstdlib),
            None => {
                if cfg!(target_os = "linux") {
                    // Default to GNU libstdc++ on Linux. Can be overwritten through
                    // `CXXSTDLIB` variable on distributions using LLVM as default
                    // toolchain.
                    Self::Single(b"stdc++")
                } else if cfg!(target_os = "macos") {
                    // Default to LLVM libc++ on macOS, where LLVM is the default
                    // toolchain.
                    if cfg!(feature = "llvm-link-static") {
                        // Static LLVM libc++ has two files - libc++.a and libc++abi.a.
                        Self::Multiple(&[b"c++", b"c++abi"])
                    } else {
                        // Shared LLVM libc++ has one file.
                        Self::Single(b"c++")
                    }
                } else {
                    // Fall back to GNU libstdc++ on all other platforms. Again,
                    // can be overwritten through `CXXSTDLIB`.
                    Self::Single(b"stdc++")
                }
            }
        })
    }

    fn iter(&self) -> impl Iterator<Item = &[u8]> {
        match self {
            Self::EnvVar(p) => CxxstdlibsIter::Parsed(p.as_bytes().split(|b| *b == b',')),
            Self::Single(s) => {
                CxxstdlibsIter::Single(iter::once(
                    // Coerce `&&[u8]` to `&[u8]`.
                    *s,
                ))
            }
            Self::Multiple(m) => CxxstdlibsIter::Multiple(
                m.iter()
                    // Coerce `&&[u8]` to `&[u8]`.
                    .copied(),
            ),
        }
    }

    fn iter_static_filenames(&self) -> impl Iterator<Item = OsString> {
        self.iter().map(|lib| {
            let mut filename = OsString::from("lib");
            filename.push(OsStr::from_bytes(lib));
            filename.push(".a");
            filename
        })
    }
}

impl Display for Cxxstdlibs<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvVar(p) => {
                Display::fmt(&p.display(), f)?;
            }
            Self::Single(s) => Display::fmt(&OsStr::from_bytes(s).display(), f)?,
            Self::Multiple(m) => {
                f.write_str("[")?;
                for (i, lib) in m.iter().enumerate() {
                    if i != 0 {
                        write!(f, ", ")?;
                    }
                    Display::fmt(&OsStr::from_bytes(lib).display(), f)?;
                }
                f.write_str("]")?;
            }
        }
        Ok(())
    }
}

enum CxxstdlibsIter<P, S, M> {
    Parsed(P),
    Single(S),
    Multiple(M),
}

impl<'a, P, S, M> Iterator for CxxstdlibsIter<P, S, M>
where
    P: Iterator<Item = &'a [u8]>,
    S: Iterator<Item = &'a [u8]>,
    M: Iterator<Item = &'a [u8]>,
{
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Parsed(p) => p.next(),
            Self::Single(s) => s.next(),
            Self::Multiple(m) => m.next(),
        }
    }
}

/// Checks whether the given environment variable `env_var` exists and if yes,
/// emits its content as a search path for the linker.
///
/// Returns a boolean indicating whether the variable was found
fn emit_search_path_if_defined(
    stdout: &mut io::StdoutLock<'_>,
    env_var: &str,
) -> anyhow::Result<bool> {
    writeln!(stdout, "cargo:rerun-if-env-changed={env_var}")?;
    match env::var_os(env_var) {
        Some(path) => {
            write_bytes!(
                stdout,
                "cargo:rustc-link-search=",
                path.as_os_str().as_bytes(),
            )?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn target_architecture_from_env() -> anyhow::Result<Architecture> {
    const CARGO_CFG_TARGET_ARCH: &str = "CARGO_CFG_TARGET_ARCH";
    let arch = env::var_os(CARGO_CFG_TARGET_ARCH).ok_or_else(|| {
        anyhow::anyhow!(
            "`{CARGO_CFG_TARGET_ARCH}` is not set, cannot determine the target architecture"
        )
    })?;
    match arch.as_bytes() {
        b"aarch64" => Ok(Architecture::Aarch64),
        b"aarch64_ilp32" => Ok(Architecture::Aarch64_Ilp32),
        b"alpha" => Ok(Architecture::Alpha),
        b"arm" => Ok(Architecture::Arm),
        b"avr" => Ok(Architecture::Avr),
        b"bpf" => Ok(Architecture::Bpf),
        b"csky" => Ok(Architecture::Csky),
        b"loongarch32" => Ok(Architecture::LoongArch32),
        b"loongarch64" => Ok(Architecture::LoongArch64),
        b"m68k" => Ok(Architecture::M68k),
        b"mips" => Ok(Architecture::Mips),
        b"mips64" => Ok(Architecture::Mips64),
        b"msp430" => Ok(Architecture::Msp430),
        b"powerpc" => Ok(Architecture::PowerPc),
        b"powerpc64" => Ok(Architecture::PowerPc64),
        b"riscv32" => Ok(Architecture::Riscv32),
        b"riscv64" => Ok(Architecture::Riscv64),
        b"s390x" => Ok(Architecture::S390x),
        b"sbf" => Ok(Architecture::Sbf),
        b"sparc" => Ok(Architecture::Sparc),
        b"sparc64" => Ok(Architecture::Sparc64),
        b"wasm32" => Ok(Architecture::Wasm32),
        b"wasm64" => Ok(Architecture::Wasm64),
        b"xtensa" => Ok(Architecture::Xtensa),
        b"x86" => Ok(Architecture::I386),
        b"x86_64" => Ok(Architecture::X86_64),
        _ => Err(anyhow::anyhow!(
            "`{CARGO_CFG_TARGET_ARCH}` references unknown architecture `{}`",
            arch.display()
        )),
    }
}

fn archive_member_object<'a>(
    member: object::read::Result<object::read::archive::ArchiveMember<'a>>,
    archive_path: &Path,
    archive_data: &'a [u8],
) -> anyhow::Result<(&'a OsStr, object::File<'a>)> {
    let member = member.with_context(|| {
        format!(
            "failed to process the member of library archive {}",
            archive_path.display()
        )
    })?;
    let member_name = OsStr::from_bytes(member.name());
    let member_data = member.data(archive_data).with_context(|| {
        format!(
            "failed to read data of static library archive member {} in {}",
            member_name.display(),
            archive_path.display()
        )
    })?;
    let obj = object::File::parse(member_data).with_context(|| {
        format!(
            "failed to parse data of static library archive member {} as object file",
            member_name.display(),
        )
    })?;
    Ok((member_name, obj))
}

fn library_matches_architecture(path: &Path, target_arch: &Architecture) -> anyhow::Result<bool> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) => {
            return if err.kind() == io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(err).with_context(|| format!("failed to read library {}", path.display()))
            };
        }
    };
    let archive = ArchiveFile::parse(data.as_slice())
        .with_context(|| format!("failed to parse library archive {}", path.display()))?;
    for member in archive.members() {
        let (_member_name, obj) = archive_member_object(member, path, data.as_slice())?;
        if obj.architecture() == *target_arch {
            return Ok(true);
        }
    }
    Ok(false)
}

fn binary_matches_architecture(path: &Path, target_arch: &Architecture) -> anyhow::Result<bool> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read binary {}", path.display()));
        }
    };
    let obj = object::File::parse(data.as_slice())
        .with_context(|| format!("failed to parse binary {} as object file", path.display()))?;
    Ok(obj.architecture() == *target_arch)
}

enum Paths<'a> {
    LdPaths(&'a OsStr),
    Slice(&'a [PathBuf]),
}

impl Paths<'_> {
    fn display(&self) -> PathsDisplay<'_> {
        match self {
            Self::LdPaths(ld_paths) => PathsDisplay::LdPaths(ld_paths.display()),
            Self::Slice(slice) => PathsDisplay::Slice(PathBufSliceDisplay(slice)),
        }
    }
}

enum PathsDisplay<'a> {
    LdPaths(os_str::Display<'a>),
    Slice(PathBufSliceDisplay<'a>),
}

impl Display for PathsDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LdPaths(display) => display.fmt(f),
            Self::Slice(display) => display.fmt(f),
        }
    }
}

pub struct PathBufSliceDisplay<'a>(&'a [PathBuf]);

impl Display for PathBufSliceDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, p) in self.0.iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }
            p.display().fmt(f)?;
        }
        write!(f, "]")
    }
}

fn check_library<S: Display>(
    stdout: &mut io::StdoutLock<'_>,
    ld_paths: Paths<'_>,
    library: S,
    paths: Option<Vec<PathBuf>>,
) -> anyhow::Result<()> {
    if let Some(paths) = paths {
        match paths.as_slice() {
            [] => {
                anyhow::bail!(
                    "could not find {library} in any of the following directories: {}",
                    ld_paths.display()
                );
            }
            [_] => {}
            paths => {
                let mut hashes = std::collections::HashMap::new();
                let mut buffer = [0; 8 * 1024];
                for path in paths {
                    use std::{hash::Hasher as _, io::Read as _};
                    let mut hasher = std::hash::DefaultHasher::new();
                    let mut file = fs::File::open(path)
                        .with_context(|| format!("failed to open file {}", path.display()))?;
                    loop {
                        let n = file
                            .read(&mut buffer)
                            .with_context(|| format!("failed to read file {}", path.display()))?;
                        if n == 0 {
                            break;
                        }
                        hasher.write(&buffer[..n]);
                    }
                    hashes
                        .entry(hasher.finish())
                        .or_insert_with(Vec::new)
                        .push(path);
                }
                if hashes.len() > 1 {
                    write!(
                        stdout,
                        "cargo:warning={library} was found in multiple locations: "
                    )?;
                    for (i, (hash, paths)) in hashes.iter().enumerate() {
                        if i != 0 {
                            write!(stdout, ", ")?;
                        }
                        write!(stdout, "[")?;
                        for (i, path) in paths.iter().enumerate() {
                            if i != 0 {
                                write!(stdout, ", ")?;
                            }
                            write!(stdout, "{}", path.display())?;
                        }
                        write!(stdout, "]=0x{hash:x}")?;
                    }
                    writeln!(stdout)?;
                }
            }
        }
    }
    Ok(())
}
/// Points cargo to static library names of LLVM and dependencies needed by
/// LLVM.
///
/// That includes figuring out which libraries LLVM depends on:
///
/// - Standard C++ library (GNU stdc++ or LLVM libc++).
/// - zlib (optional).
/// - zstd (optional).
///
/// It's necessary, because static libraries have no equivalent of `DT_NEEDED`
/// entries. They come with undefined symbols that must be filled in by other
/// libraries at link time. Since static archives do not explicitly express
/// which additional libraries are required, we have to determine that set
/// ourselves using the undefined symbols, and instruct Cargo to link them.
fn link_llvm_static(
    stdout: &mut io::StdoutLock<'_>,
    target_architecture: &Architecture,
    llvm_lib_dirs: &[PathBuf],
) -> anyhow::Result<()> {
    // Link the LLVM library files (that match the target architecture) and find
    // libLLVMSupport.a for the inspection below.
    const LIBLLVM_SUPPORT: &str = "libLLVMSupport.a";
    let mut libllvm_support = Vec::new();
    for llvm_lib_dir in llvm_lib_dirs {
        let dir_entries = fs::read_dir(llvm_lib_dir)
            .with_context(|| format!("failed to read directory {}", llvm_lib_dir.display()))?;
        let mut found_first = false;
        for entry in dir_entries {
            let entry = entry.with_context(|| {
                format!(
                    "failed to read entry of the directory {}",
                    llvm_lib_dir.display()
                )
            })?;
            let file_name = entry.file_name();
            if file_name == LIBLLVM_SUPPORT {
                libllvm_support.push(entry.path());
            }
            let file_name = file_name.as_bytes();
            let Some(trimmed) = file_name
                .strip_prefix(b"libLLVM")
                .and_then(|name| name.strip_suffix(b".a"))
            else {
                continue;
            };
            if !library_matches_architecture(&entry.path(), target_architecture)? {
                continue;
            }

            if !found_first {
                write_bytes!(
                    stdout,
                    b"cargo:rustc-link-search=",
                    llvm_lib_dir.as_os_str().as_bytes(),
                )?;
                found_first = true;
            }
            write_bytes!(stdout, "cargo:rustc-link-lib=static=LLVM", trimmed)?;
        }
    }
    check_library(
        stdout,
        Paths::Slice(llvm_lib_dirs),
        LIBLLVM_SUPPORT,
        Some(libllvm_support.clone()),
    )?;
    let libllvm_support = libllvm_support
        .into_iter()
        .next()
        .expect("check_library ensures at least one libLLVMSupport.a candidate is present");

    // Static libraries have no metadata indicating a dependency on other
    // libraries. Given that zlib and zstd might or might be not enabled in
    // different LLVM builds, check whether libLLVMSupport references their
    // symbols.
    let (mut needs_zlib, mut needs_zstd) = (false, false);
    let data = fs::read(&libllvm_support)
        .with_context(|| format!("failed to read library {}", libllvm_support.display()))?;
    let archive = ArchiveFile::parse(data.as_slice()).with_context(|| {
        format!(
            "failed to parse library archive {}",
            libllvm_support.display()
        )
    })?;
    'outer: for member in archive.members() {
        let (member_name, obj) = archive_member_object(member, &libllvm_support, data.as_slice())?;
        for symbol in obj.symbols() {
            if symbol.is_undefined() {
                let sym_name = symbol.name().with_context(|| {
                    format!(
                        "failed to retrieve the symbol name in static library archive member {}",
                        member_name.display()
                    )
                })?;
                if sym_name.contains("crc32") {
                    needs_zlib = true;
                } else if sym_name.contains("ZSTD") {
                    needs_zstd = true;
                }
                if needs_zlib && needs_zstd {
                    break 'outer;
                }
            }
        }
    }

    let cxxstdlibs = Cxxstdlibs::new(stdout)?;

    // Find directories with static libraries we're interested in:
    // - C++ standard library
    // - zlib (if needed)
    // - zstd (if needed)

    // Check whether custom paths were provided. If yes, point the linker to
    // them.
    let cxxstdlib_found = emit_search_path_if_defined(stdout, "CXXSTDLIB_PATH")?;
    let zlib_found = needs_zlib && emit_search_path_if_defined(stdout, "ZLIB_PATH")?;
    let zstd_found = needs_zstd && emit_search_path_if_defined(stdout, "LIBZSTD_PATH")?;

    if !cxxstdlib_found || (needs_zlib && !zlib_found) || (needs_zstd && !zstd_found) {
        // Unfortunately, Rust/cargo don't look for static libraries in system
        // directories, like C compilers do, so we had to implement the logic
        // of searching for them ourselves.

        // Use C compiler to retrieve the system library paths with
        // `-print-search-dirs` option.
        const CC: &str = "CC";
        const RUSTC_LINKER: &str = "RUSTC_LINKER";
        /// Executes `maybe_cc` with `-print-search-dirs` argument. On success,
        /// returns the output.
        fn print_search_dirs<'a>(
            stdout: &mut io::StdoutLock<'_>,
            var: Option<&str>,
        ) -> anyhow::Result<Option<(Cow<'a, OsStr>, Vec<u8>)>> {
            let maybe_cc = match var {
                Some(var) => {
                    writeln!(stdout, "cargo:rerun-if-env-changed={var}")?;
                    match env::var_os(var).map(Cow::Owned) {
                        Some(var) => var,
                        // If the environment variable is not defined, proceed with
                        // the next candidate.
                        None => return Ok(None),
                    }
                }
                // Use `cc` as the last option. Pretty much all UNIX-like operating
                // systems provide `/usr/bin/cc` as a symlink to the default
                // compiler (either clang or gcc).
                None => Cow::Borrowed(OsStr::from_bytes(b"cc")),
            };
            let mut cmd = Command::new(&maybe_cc);
            let linker_output = cmd
                .arg("-print-search-dirs")
                .output()
                .with_context(|| format!("failed to run {cmd:?}"))?;
            if linker_output.status.success() {
                Ok(Some((maybe_cc, linker_output.stdout)))
            } else {
                // We don't return an error here, instead we log a warning and
                // proceed with trying next candidates.
                // The failure here usually means that `maybe_cc` is a regular
                // linker (e.g. rust-lld, bfd, lld), not a C compiler (e.g.
                // clang, gcc).
                if let Some(var) = var {
                    writeln!(
                        stdout,
                        "cargo:warning=`command `{cmd:?}` (specified by environment variable
{var}: {}) failed: {}",
                        maybe_cc.display(),
                        linker_output.status
                    )?;
                } else {
                    writeln!(
                        stdout,
                        "cargo:warning=`command `{cmd:?}` failed: {}",
                        linker_output.status
                    )?;
                }
                Ok(None)
            }
        }
        let (cc, linker_stdout) = [
            // Try to use the `CC` environment variable, allowing users to
            // overwrite defaults.
            Some(CC),
            // Try to retrieve it from `RUSTC_LINKER` environment
            // variable. It's defined by cargo only if `-C linker` option is
            // provided. Users are not supposed to define this variable on
            // their own.
            // Note that it might be either a C compiler or a regular linker.
            Some(RUSTC_LINKER),
            None,
        ]
        .into_iter()
        .find_map(|var| print_search_dirs(stdout, var).transpose())
        .transpose()?
        .ok_or_else(|| {
            anyhow!(
                "could not find C compiler capable of reporting library search paths,
(with `-print-search-dirs`), consider setting `CC` environment variable pointing
to an appropriate compiler"
            )
        })?;
        let ld_paths = linker_stdout
            .split(|&b| b == b'\n')
            .find_map(|line| line.strip_prefix(b"libraries: ="))
            .ok_or_else(|| {
                anyhow!(
                    "failed to find library paths in the output of `{} -print-search-dirs`: {}",
                    cc.display(),
                    OsStr::from_bytes(&linker_stdout).display()
                )
            })?;
        let ld_paths = OsStr::from_bytes(ld_paths);

        // Find directories with static libraries we're interested in:
        // - C++ standard library
        // - zlib (if needed)
        // - zstd (if needed)
        const ZLIB: &str = "libz.a";
        const ZSTD: &str = "libzstd.a";
        let mut cxxstdlib_paths = (!cxxstdlib_found).then(Vec::new);
        let mut zlib_paths = (needs_zlib && !zlib_found).then(Vec::new);
        let mut zstd_paths = (needs_zstd && !zstd_found).then(Vec::new);
        for ld_path in env::split_paths(ld_paths) {
            let mut found_any = false;
            if let Some(ref mut cxxstdlib_paths) = cxxstdlib_paths {
                for cxxstdlib in cxxstdlibs.iter_static_filenames() {
                    let cxxstdlib_path = ld_path.join(cxxstdlib);
                    if library_matches_architecture(&cxxstdlib_path, target_architecture)? {
                        cxxstdlib_paths.push(cxxstdlib_path);
                        found_any = true;
                    }
                }
            }
            if let Some(ref mut zlib_paths) = zlib_paths {
                let zlib_path = ld_path.join(ZLIB);
                if library_matches_architecture(&zlib_path, target_architecture)? {
                    zlib_paths.push(zlib_path);
                    found_any = true;
                }
            }
            if let Some(ref mut zstd_paths) = zstd_paths {
                let zstd_path = ld_path.join(ZSTD);
                if library_matches_architecture(&zstd_path, target_architecture)? {
                    zstd_paths.push(zstd_path);
                    found_any = true;
                }
            }
            if found_any {
                write_bytes!(
                    stdout,
                    "cargo:rustc-link-search=",
                    ld_path.as_os_str().as_bytes(),
                )?;
            }
        }

        check_library(
            stdout,
            Paths::LdPaths(ld_paths),
            &cxxstdlibs,
            cxxstdlib_paths,
        )?;
        check_library(stdout, Paths::LdPaths(ld_paths), ZLIB, zlib_paths)?;
        check_library(stdout, Paths::LdPaths(ld_paths), ZSTD, zstd_paths)?;
    }

    for cxxstdlib in cxxstdlibs.iter() {
        write_bytes!(stdout, "cargo:rustc-link-lib=static=", cxxstdlib)?;
    }
    if needs_zlib {
        write_bytes!(stdout, "cargo:rustc-link-lib=static=z")?;
    }
    if needs_zstd {
        write_bytes!(stdout, "cargo:rustc-link-lib=static=zstd")?;
    }

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(
    stdout: &mut io::StdoutLock<'_>,
    target_architecture: &Architecture,
    llvm_lib_dirs: &[PathBuf],
) -> anyhow::Result<()> {
    const LIB_LLVM: &[u8] = b"libLLVM";
    #[cfg(target_os = "macos")]
    const DYLIB_EXT: &[u8] = b".dylib";
    #[cfg(not(target_os = "macos"))]
    const DYLIB_EXT: &[u8] = b".so";

    let mut library_dirs = Vec::new();
    let mut library_names = Vec::new();
    let mut library_paths = Vec::new();
    for llvm_lib_dir in llvm_lib_dirs {
        let dir_entries = fs::read_dir(llvm_lib_dir).with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        for entry in dir_entries {
            let entry = entry.with_context(|| {
                format!(
                    "failed to read entry of the directory {}",
                    llvm_lib_dir.display()
                )
            })?;
            let mut file_name = entry.file_name().into_encoded_bytes();
            if file_name.starts_with(LIB_LLVM) && file_name.ends_with(DYLIB_EXT) {
                if !binary_matches_architecture(&entry.path(), target_architecture)? {
                    continue;
                }
                drop(file_name.drain((file_name.len() - DYLIB_EXT.len())..));
                drop(file_name.drain(..LIB_LLVM.len()));
                // SAFETY: `file_name` originates from `OsString::into_encoded_bytes`.
                // Since then, it was only trimmed.
                let library = unsafe { OsString::from_encoded_bytes_unchecked(file_name) };
                library_dirs.push(llvm_lib_dir.clone());
                library_names.push(library);
                library_paths.push(entry.path());
            }
        }
    }
    check_library(
        stdout,
        Paths::Slice(llvm_lib_dirs),
        "libLLVM",
        Some(library_paths),
    )?;
    let llvm_lib_dir = library_dirs
        .first()
        .expect("`check_library` ensures at least one libLLVM candidate is present");
    let library = library_names
        .first()
        .expect("`check_library` ensures at least one libLLVM candidate is present");
    write_bytes!(
        stdout,
        b"cargo:rustc-link-search=",
        llvm_lib_dir.as_os_str().as_bytes(),
    )?;
    write_bytes!(
        stdout,
        "cargo:rustc-link-lib=dylib=LLVM",
        library.as_bytes(),
    )?;

    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
fn main() -> anyhow::Result<()> {
    let link_fn = if cfg!(feature = "no-llvm-linking") {
        if cfg!(feature = "llvm-link-static") {
            anyhow::bail!("`no-llvm-linking` and `llvm-link-static` are mutually exclusive");
        }
        return Ok(());
    } else if cfg!(feature = "llvm-link-static") {
        link_llvm_static
    } else {
        link_llvm_dynamic
    };

    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    // If `LLVM_PREFIX` variable is not provided, find the directory with LLVM
    // libraries by assuming it's `lib/` inside a prefix where `llvm-config`
    // lives.
    const LLVM_PREFIX: &str = "LLVM_PREFIX";
    const PATH: &str = "PATH";
    writeln!(stdout, "cargo:rerun-if-env-changed={LLVM_PREFIX}")?;
    writeln!(stdout, "cargo:rerun-if-env-changed={PATH}")?;
    let (var_name, paths_os) = env::var_os(LLVM_PREFIX)
        .map(|mut p| {
            p.push("/bin");
            (LLVM_PREFIX, p)
        })
        .or_else(|| env::var_os(PATH).map(|p| (PATH, p)))
        .ok_or_else(|| anyhow!("neither {LLVM_PREFIX} nor {PATH} is set"))?;
    let target_architecture = target_architecture_from_env()?;
    let llvm_config = env::split_paths(&paths_os)
        .find_map(|dir| {
            let candidate = Path::new(&dir).join("llvm-config");
            match binary_matches_architecture(&candidate, &target_architecture) {
                Ok(binary_matches) => binary_matches.then_some(Ok(candidate)),
                Err(err) => Some(Err(err)),
            }
        })
        .transpose()?
        .ok_or_else(|| {
            anyhow!(
                "could not find llvm-config targeting architecture {target_architecture:?} in directories specified by environment
variable `{var_name}` {}",
                paths_os.display(),
            )
        })?;
    let llvm_basedir = llvm_config
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            anyhow!(
                "llvm-config location has no parent: {}",
                llvm_config.display()
            )
        })?;
    fn maybe_canonicalize_llvm_lib_dir(llvm_lib_dir: PathBuf) -> anyhow::Result<Option<PathBuf>> {
        match fs::canonicalize(&llvm_lib_dir) {
            Ok(path) => Ok(Some(path)),
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(err).with_context(|| {
                        format!(
                            "failed to canonicalize LLVM lib directory {}",
                            llvm_lib_dir.display()
                        )
                    })
                }
            }
        }
    }
    let llvm_lib_dirs = ["lib", "lib64"]
        .iter()
        .filter_map(|lib_dir| {
            maybe_canonicalize_llvm_lib_dir(llvm_basedir.join(lib_dir)).transpose()
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    link_fn(&mut stdout, &target_architecture, &llvm_lib_dirs)
}
