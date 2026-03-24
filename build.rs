use std::{
    borrow::Cow,
    env,
    ffi::{OsStr, OsString},
    fmt::{self, Display, Formatter},
    fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, anyhow};
use object::{AddressSize, Architecture};

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

/// Parses an [`Architecture`] from the `CARGO_CFG_TARGET_ARCH` variable, that
/// determines the target architecture of the build.
fn target_architecture_from_env() -> anyhow::Result<(Architecture, OsString)> {
    const CARGO_CFG_TARGET_ARCH: &str = "CARGO_CFG_TARGET_ARCH";
    let arch = env::var_os(CARGO_CFG_TARGET_ARCH).ok_or_else(|| {
        anyhow::anyhow!(
            "`{CARGO_CFG_TARGET_ARCH}` is not set, cannot determine the target architecture"
        )
    })?;
    let parsed_arch = match arch.as_bytes() {
        b"aarch64" => Architecture::Aarch64,
        b"aarch64_ilp32" => Architecture::Aarch64_Ilp32,
        b"alpha" => Architecture::Alpha,
        b"arm" => Architecture::Arm,
        b"avr" => Architecture::Avr,
        b"bpf" => Architecture::Bpf,
        b"csky" => Architecture::Csky,
        b"loongarch32" => Architecture::LoongArch32,
        b"loongarch64" => Architecture::LoongArch64,
        b"m68k" => Architecture::M68k,
        b"mips" => Architecture::Mips,
        b"mips64" => Architecture::Mips64,
        b"msp430" => Architecture::Msp430,
        b"powerpc" => Architecture::PowerPc,
        b"powerpc64" => Architecture::PowerPc64,
        b"riscv32" => Architecture::Riscv32,
        b"riscv64" => Architecture::Riscv64,
        b"s390x" => Architecture::S390x,
        b"sbf" => Architecture::Sbf,
        b"sparc" => Architecture::Sparc,
        b"sparc64" => Architecture::Sparc64,
        b"wasm32" => Architecture::Wasm32,
        b"wasm64" => Architecture::Wasm64,
        b"xtensa" => Architecture::Xtensa,
        b"x86" => Architecture::I386,
        b"x86_64" => Architecture::X86_64,
        _ => anyhow::bail!(
            "`{CARGO_CFG_TARGET_ARCH}` references unknown architecture `{}`",
            arch.display()
        ),
    };
    Ok((parsed_arch, arch))
}

/// Finds an existing library directory among provided `candidates` in the
/// `basedir`.
fn find_libdir<P>(
    stdout: &mut io::StdoutLock<'_>,
    basedir: &Path,
    candidates: &[P],
) -> anyhow::Result<PathBuf>
where
    P: AsRef<Path> + fmt::Debug,
{
    candidates
        .iter()
        .find_map(|candidate| {
            || -> anyhow::Result<Option<PathBuf>> {
                let candidate = basedir.join(candidate);
                if candidate.exists() {
                    Ok(Some(candidate))
                } else {
                    write_bytes!(
                        stdout,
                        "cargo:warning=directory does not exist: ",
                        candidate.as_os_str().as_bytes()
                    )?;
                    Ok(None)
                }
            }()
            .transpose()
        })
        .transpose()?
        .ok_or_else(|| anyhow!("none of the candidate directories exist: {candidates:?}"))
}

/// Checks whether the given environment variable `env_var` exists and if yes,
/// emits its content as a search path for the linker.
///
/// Returns a boolean indicating whether the variable was found.
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

/// Points cargo to static library names of LLVM and dependencies needed by
/// LLVM.
///
/// That includes figuring out which libraries LLVM depends on:
///
/// - Standard C++ library (GNU stdc++ or LLVM libc++).
/// - zlib.
/// - zstd.
///
/// It's necessary, because static libraries have no equivalent of `DT_NEEDED`
/// entries. They come with undefined symbols that must be filled in by other
/// libraries at link time. Since static archives do not explicitly express
/// which additional libraries are required, we have to determine that set
/// ourselves using the undefined symbols, and instruct Cargo to link them.
fn link_llvm_static(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: &Path) -> anyhow::Result<()> {
    // Link the library files found inside the directory.
    let dir_entries = fs::read_dir(llvm_lib_dir)
        .with_context(|| format!("failed to read directory {}", llvm_lib_dir.display()))?;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.as_bytes();
        let Some(trimmed) = file_name
            .strip_prefix(b"libLLVM")
            .and_then(|name| name.strip_suffix(b".a"))
        else {
            continue;
        };

        write_bytes!(stdout, "cargo:rustc-link-lib=static=LLVM", trimmed)?;
    }

    let cxxstdlibs = Cxxstdlibs::new(stdout)?;

    // Find directories with static libraries we're interested in:
    // - C++ standard library
    // - zlib (if needed)
    // - zstd (if needed)

    // Check whether custom paths were provided. If yes, point the linker to
    // them.
    let cxxstdlib_found = emit_search_path_if_defined(stdout, "CXXSTDLIB_PATH")?;
    let zlib_found = emit_search_path_if_defined(stdout, "ZLIB_PATH")?;
    let zstd_found = emit_search_path_if_defined(stdout, "LIBZSTD_PATH")?;

    if !cxxstdlib_found || !zlib_found || !zstd_found {
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
        let mut zlib_paths = (!zlib_found).then(Vec::new);
        let mut zstd_paths = (!zstd_found).then(Vec::new);
        for ld_path in env::split_paths(ld_paths) {
            let mut found_any = false;
            if let Some(ref mut cxxstdlib_paths) = cxxstdlib_paths {
                for cxxstdlib in cxxstdlibs.iter_static_filenames() {
                    let cxxstdlib_path = ld_path.join(cxxstdlib);
                    if cxxstdlib_path.try_exists().with_context(|| {
                        format!("failed to inspect the file {}", cxxstdlib_path.display(),)
                    })? {
                        cxxstdlib_paths.push(cxxstdlib_path);
                        found_any = true;
                    }
                }
            }
            if let Some(ref mut zlib_paths) = zlib_paths {
                let zlib_path = ld_path.join(ZLIB);
                if zlib_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", zlib_path.display())
                })? {
                    zlib_paths.push(zlib_path);
                    found_any = true;
                }
            }
            if let Some(ref mut zstd_paths) = zstd_paths {
                let zstd_path = ld_path.join("libzstd.a");
                if zstd_path.try_exists().with_context(|| {
                    format!("failed to inspect the file {}", zstd_path.display())
                })? {
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

        fn check_library<S: Display>(
            stdout: &mut io::StdoutLock<'_>,
            ld_paths: &OsStr,
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
                            let mut file = fs::File::open(path).with_context(|| {
                                format!("failed to open file {}", path.display())
                            })?;
                            loop {
                                let n = file.read(&mut buffer).with_context(|| {
                                    format!("failed to read file {}", path.display())
                                })?;
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
        check_library(stdout, ld_paths, &cxxstdlibs, cxxstdlib_paths)?;
        check_library(stdout, ld_paths, ZLIB, zlib_paths)?;
        check_library(stdout, ld_paths, ZSTD, zstd_paths)?;
    }

    for cxxstdlib in cxxstdlibs.iter() {
        write_bytes!(stdout, "cargo:rustc-link-lib=static=", cxxstdlib)?;
    }
    write_bytes!(stdout, "cargo:rustc-link-lib=static=z")?;
    write_bytes!(stdout, "cargo:rustc-link-lib=static=zstd")?;

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: &Path) -> anyhow::Result<()> {
    const LIB_LLVM: &[u8] = b"libLLVM";
    const CARGO_CFG_TARGET_OS: &str = "CARGO_CFG_TARGET_OS";
    let dylib_ext = match env::var_os(CARGO_CFG_TARGET_OS)
        .ok_or_else(|| {
            anyhow!(
                "{CARGO_CFG_TARGET_OS} is not defined, cannot determine the target architecture"
            )
        })?
        .as_encoded_bytes()
    {
        b"macos" => b".dylib".as_slice(),
        _ => b".so".as_slice(),
    };

    let dir_entries = fs::read_dir(llvm_lib_dir).with_context(|| {
        format!(
            "failed to read entry of the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let libraries = dir_entries
        .filter_map(|entry| {
            entry
                .with_context(|| {
                    format!(
                        "failed to read entry of the directory {}",
                        llvm_lib_dir.display()
                    )
                })
                .map(|entry| {
                    let mut file_name = entry.file_name().into_encoded_bytes();
                    if file_name.starts_with(LIB_LLVM) && file_name.ends_with(dylib_ext) {
                        drop(file_name.drain((file_name.len() - dylib_ext.len())..));
                        drop(file_name.drain(..LIB_LLVM.len()));
                        // SAFETY: `file_name` originates from `OsString::into_encoded_bytes`.
                        // Since then, it was only trimmed.
                        Some(unsafe { OsString::from_encoded_bytes_unchecked(file_name) })
                    } else {
                        None
                    }
                })
                .transpose()
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let library = match libraries.as_slice() {
        [] => {
            anyhow::bail!(
                "could not find dynamic libLLVM in the directory {}",
                llvm_lib_dir.display()
            );
        }
        [library] => library,
        libraries @ [library, ..] => {
            writeln!(
                stdout,
                "cargo:warning=found multiple libLLVM files in directory {}:
{libraries:?}",
                llvm_lib_dir.display()
            )?;
            library
        }
    };
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
    let llvm_config = env::split_paths(&paths_os)
        .find_map(|dir| {
            let candidate = Path::new(&dir).join("llvm-config");
            candidate
                .try_exists()
                .with_context(|| format!("failed to inspect the file {}", candidate.display()))
                .map(|exists| exists.then_some(candidate))
                .transpose()
        })
        .transpose()?
        .ok_or_else(|| {
            anyhow!(
                "could not find llvm-config in directories specified by environment
variable `{var_name}` {}",
                paths_os.display()
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
    let (target_arch, target_arch_ostr) = target_architecture_from_env()?;
    let target_address_size = target_arch.address_size().ok_or_else(|| {
        anyhow!(
            "address size of target architecture {} is unknown",
            target_arch_ostr.display()
        )
    })?;
    // Support systems that provide libraries for multiple address sizes,
    // usually 64-bit and 32-bit, to allow 32-bit applications on 64-bit hosts.
    // Prefer directories with an explicit address size, such as `lib32` or
    // `lib64` for the target, then fall back to `lib`.
    let (target_bits, lib_dir_candidates) = match target_address_size {
        AddressSize::U32 => (32, ["lib32", "lib"]),
        AddressSize::U64 => (64, ["lib64", "lib"]),
        _ => anyhow::bail!(
            "target {} with address size {target_address_size:?} is not supported",
            target_arch_ostr.display()
        ),
    };
    writeln!(
        stdout,
        "cargo:warning={} is a {}-bit target, searching for LLVM library directories {lib_dir_candidates:?} in {}",
        target_arch_ostr.display(),
        target_bits,
        llvm_basedir.display(),
    )?;
    let llvm_lib_dir = find_libdir(&mut stdout, llvm_basedir, &lib_dir_candidates)
        .with_context(|| "could not find LLVM lib directory")?;

    let llvm_lib_dir = fs::canonicalize(&llvm_lib_dir).with_context(|| {
        format!(
            "failed to canonicalize LLVM lib directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let llvm_lib_dir_b = llvm_lib_dir.as_os_str().as_bytes();
    write_bytes!(stdout, b"cargo:rustc-link-search=", llvm_lib_dir_b,)?;
    write_bytes!(
        stdout,
        b"cargo:warning=found LLVM library directory: ",
        llvm_lib_dir_b,
    )?;

    link_fn(&mut stdout, &llvm_lib_dir)
}
