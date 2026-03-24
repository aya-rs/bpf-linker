use std::{
    env,
    ffi::OsString,
    fmt, fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
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
/// emits its content as a `-L` search path for the linker.
fn emit_search_path_if_defined(
    stdout: &mut io::StdoutLock<'_>,
    env_var: &str,
) -> anyhow::Result<()> {
    writeln!(stdout, "cargo:rerun-if-env-changed={env_var}")?;
    if let Some(path) = env::var_os(env_var) {
        write_bytes!(
            stdout,
            "cargo:rustc-link-arg=-L",
            path.as_os_str().as_bytes()
        )?;
    }
    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
fn main() -> anyhow::Result<()> {
    if cfg!(feature = "no-llvm-linking") {
        if cfg!(feature = "llvm-link-static") {
            anyhow::bail!("`no-llvm-linking` and `llvm-link-static` are mutually exclusive");
        }
        return Ok(());
    }

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
    write_bytes!(
        stdout,
        b"cargo:warning=found LLVM library directory: ",
        llvm_lib_dir_b
    )?;
    write_bytes!(stdout, b"cargo:rustc-link-arg=-L", llvm_lib_dir_b)?;

    if cfg!(feature = "llvm-link-static") {
        // Link LLVM and its dependencies statically:
        // - Standard C++ library (GNU stdc++ or LLVM libc++).
        // - zlib.
        // - zstd.
        writeln!(stdout, "cargo:rustc-link-arg=-Wl,-static")?;

        // Link the library files found inside the directory.
        let dir_entries = fs::read_dir(&llvm_lib_dir)
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

            write_bytes!(stdout, "cargo:rustc-link-arg=-lLLVM", trimmed)?;
        }

        let cxxstdlibs = Cxxstdlibs::new(&mut stdout)?;

        // Let the final linker resolve libc++ wrappers and any directory search
        // semantics. Explicit override directories are forwarded via `-L`.
        emit_search_path_if_defined(&mut stdout, "CXXSTDLIB_PATH")?;
        emit_search_path_if_defined(&mut stdout, "ZLIB_PATH")?;
        emit_search_path_if_defined(&mut stdout, "LIBZSTD_PATH")?;

        for cxxstdlib in cxxstdlibs.iter() {
            write_bytes!(stdout, "cargo:rustc-link-arg=-l", cxxstdlib)?;
        }
        writeln!(stdout, "cargo:rustc-link-arg=-lz")?;
        writeln!(stdout, "cargo:rustc-link-arg=-lzstd")?;
    } else {
        // Link against shared LLVM. Unlike the static case, its dependencies
        // are discovered via `DT_NEEDED` entries in the shared object.
        write_bytes!(stdout, "cargo:rustc-link-arg=-lLLVM")?;
    }

    Ok(())
}
