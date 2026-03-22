use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::Path,
};

use anyhow::{Context as _, anyhow};
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

/// Links LLVM and its dependencies statically:
///
/// - Standard C++ library (GNU stdc++ or LLVM libc++).
/// - zlib.
/// - zstd.
fn link_llvm_static(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: &Path) -> anyhow::Result<()> {
    writeln!(stdout, "cargo:rustc-link-arg=-Wl,-static")?;

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

    // Let the final linker resolve libc++ wrappers and any directory search
    // semantics. Explicit override directories are forwarded via `-L`.
    emit_search_path_if_defined(stdout, "CXXSTDLIB_PATH")?;
    emit_search_path_if_defined(stdout, "ZLIB_PATH")?;
    emit_search_path_if_defined(stdout, "LIBZSTD_PATH")?;

    for cxxstdlib in cxxstdlibs.iter() {
        write_bytes!(stdout, "cargo:rustc-link-arg=-l", cxxstdlib)?;
    }
    writeln!(stdout, "cargo:rustc-link-arg=-lz")?;
    writeln!(stdout, "cargo:rustc-link-arg=-lzstd")?;

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(stdout: &mut io::StdoutLock<'_>) -> anyhow::Result<()> {
    write_bytes!(stdout, "cargo:rustc-link-lib=dylib=LLVM")?;

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
    let llvm_lib_dir = llvm_config
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| {
            anyhow!(
                "llvm-config location has no parent: {}",
                llvm_config.display()
            )
        })?
        .join("lib");
    let llvm_lib_dir = fs::canonicalize(&llvm_lib_dir).with_context(|| {
        format!(
            "failed to canonicalize LLVM lib directory {}",
            llvm_lib_dir.display()
        )
    })?;
    write_bytes!(
        stdout,
        b"cargo:rustc-link-search=",
        llvm_lib_dir.as_os_str().as_bytes(),
    )?;

    if cfg!(feature = "llvm-link-static") {
        link_llvm_static(&mut stdout, &llvm_lib_dir)
    } else {
        link_llvm_dynamic(&mut stdout)
    }
}
