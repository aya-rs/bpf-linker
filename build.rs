use std::{
    borrow::Cow,
    env,
    ffi::{OsStr, OsString},
    fmt::Display,
    fs,
    io::{self, Write as _},
    iter,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, anyhow};
use object::{Object as _, ObjectSymbol as _, read::archive::ArchiveFile};

macro_rules! write_bytes {
    ($dst:expr, $($bytes:expr),* $(,)?) => {
        {
            let result = (|| -> anyhow::Result<()> {
                $(
                    $dst.write_all($bytes.as_ref()).with_context(|| {
                        format!(
                            "failed to write bytes to stdout: {}",
                            OsStr::from_bytes($bytes.as_ref()).display()
                        )
                    })?;
                )*
                Ok(())
            })();

            result
        }
    };
}

enum Cxxstdlibs<'a> {
    EnvVar(OsString),
    Single(&'static [u8]),
    Multiple(&'a [&'static [u8]]),
}

impl Cxxstdlibs<'_> {
    /// Detects which standard C++ library to link.
    fn new() -> Self {
        match env::var_os("CXXSTDLIB") {
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
                    if cfg!(feature = "llvm-deps-link-static") {
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
        }
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

impl Display for Cxxstdlibs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut libs = self.iter();
        if let Some(first) = libs.next() {
            OsStr::from_bytes(first).display().fmt(f)?;
        }
        for lib in libs {
            write!(f, ", ")?;
            OsStr::from_bytes(lib).display().fmt(f)?;
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
    match env::var_os(env_var) {
        Some(path) => {
            write_bytes!(
                stdout,
                "cargo:rustc-link-search=",
                path.as_os_str().as_bytes(),
                "\n",
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
/// - zlib (optional).
/// - zstd (optional).
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

        write_bytes!(stdout, "cargo:rustc-link-lib=static=LLVM", trimmed, "\n")?;
    }

    // Static libraries have no metadata indicating a dependency on other
    // libraries. Given that zlib and zstd might or might be not enabled in
    // different LLVM builds, check whether libLLVMSupport references their
    // symbols.
    let (mut needs_zlib, mut needs_zstd) = (false, false);
    let llvm_support = llvm_lib_dir.join("libLLVMSupport.a");
    let data = fs::read(&llvm_support)
        .with_context(|| format!("failed to read library {}", llvm_support.display()))?;
    let archive = ArchiveFile::parse(data.as_slice())
        .with_context(|| format!("failed to parse library archive {}", llvm_support.display()))?;
    'outer: for member in archive.members() {
        let member = member.with_context(|| {
            format!(
                "failed to process the member of library archive {}",
                llvm_support.display()
            )
        })?;
        let member_data = member.data(data.as_slice()).with_context(|| {
            format!(
                "failed to read data of static library archive member {}",
                OsStr::from_bytes(member.name()).display()
            )
        })?;
        let obj = object::File::parse(member_data).with_context(|| {
            format!(
                "failed to parse data of static library archive member {} as object file",
                OsStr::from_bytes(member.name()).display()
            )
        })?;
        for symbol in obj.symbols() {
            if symbol.is_undefined() {
                let sym_name = symbol.name().with_context(|| {
                    format!(
                        "failed to retrieve the symbol name in static library archive member {}",
                        OsStr::from_bytes(member.name()).display()
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

    let cxxstdlibs = Cxxstdlibs::new();

    // Find directories with static libraries we're interested in:
    // - C++ standard library
    // - zlib (if needed)
    // - zstd (if needed)

    // Check whether custom paths were provided. If yes, point the linker to
    // them.
    let cxxstdlib_found = emit_search_path_if_defined(stdout, "CXXSTDLIB_PATH")?;
    let zlib_found = if needs_zlib {
        emit_search_path_if_defined(stdout, "ZLIB_PATH")?
    } else {
        false
    };
    let zstd_found = if needs_zstd {
        emit_search_path_if_defined(stdout, "LIBZSTD_PATH")?
    } else {
        false
    };

    let linkage = if cfg!(feature = "llvm-deps-link-static") {
        if !cxxstdlib_found || (needs_zlib && !zlib_found) || (needs_zstd && !zstd_found) {
            // Unfortunately, Rust/cargo don't look for static libraries in system
            // directories, like C compilers do, so we had to implement the logic
            // of searching for them ourselves.

            // Use C compiler to retrieve the system library paths.
            let cc = match env::var_os("CC") {
                Some(cc) => Cow::Owned(cc),
                None => Cow::Borrowed(OsStr::from_bytes(b"cc")),
            };
            let cc_output = Command::new(&cc)
                .arg("-print-search-dirs")
                .output()
                .with_context(|| format!("failed to run `{} -print-search-dirs`", cc.display()))?;
            if !cc_output.status.success() {
                anyhow::bail!(
                    "`{} -print-search-dirs` failed with status: {}",
                    cc.display(),
                    cc_output.status
                );
            }
            let cc_stdout = cc_output.stdout;
            let ld_paths = cc_stdout
                .split(|&b| b == b'\n')
                .find_map(|line| line.strip_prefix(b"libraries: ="))
                .ok_or_else(|| {
                    anyhow!(
                        "failed to find library paths in the output of `{} -print-search-dirs`: {}",
                        cc.display(),
                        OsStr::from_bytes(&cc_stdout).display()
                    )
                })?;
            let ld_paths = OsStr::from_bytes(ld_paths);

            // Find directories with static libraries we're interested in:
            // - C++ standard library
            // - zlib (if needed)
            // - zstd (if needed)
            let mut cxxstdlib_paths = if !cxxstdlib_found {
                Some(Vec::new())
            } else {
                None
            };
            let mut zlib_paths = if needs_zlib && !zlib_found {
                Some(Vec::new())
            } else {
                None
            };
            let mut zstd_paths = if needs_zstd && !zstd_found {
                Some(Vec::new())
            } else {
                None
            };
            for ld_path in env::split_paths(ld_paths) {
                let mut found_any = false;
                if !cxxstdlib_found && let Some(ref mut cxxstdlib_paths) = cxxstdlib_paths {
                    for cxxstdlib in cxxstdlibs.iter() {
                        let cxxstdlib_path = ld_path.join(OsStr::from_bytes(cxxstdlib));
                        if cxxstdlib_path.try_exists().with_context(|| {
                            format!("failed to inspect the file {}", cxxstdlib_path.display(),)
                        })? {
                            cxxstdlib_paths.push(cxxstdlib_path);
                            found_any = true;
                        }
                    }
                }
                if needs_zlib
                    && !zlib_found
                    && let Some(ref mut zlib_paths) = zlib_paths
                {
                    let zlib_path = ld_path.join("libz.a");
                    if zlib_path.try_exists().with_context(|| {
                        format!("failed to inspect the file {}", zlib_path.display())
                    })? {
                        zlib_paths.push(zlib_path);
                        found_any = true;
                    }
                }
                if needs_zstd
                    && !zstd_found
                    && let Some(ref mut zstd_paths) = zstd_paths
                {
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
                        io::stdout(),
                        "cargo:rustc-link-search=",
                        ld_path.as_os_str().as_bytes(),
                        "\n"
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
                            write!(
                                stdout,
                                "cargo:warning={library} was found in multiple locations: "
                            )?;
                            let mut paths = paths.iter();
                            if let Some(first) = paths.next() {
                                write_bytes!(stdout, first.as_os_str().as_bytes())?;
                            }
                            for path in paths {
                                write_bytes!(stdout, ", ", path.as_os_str().as_bytes())?;
                            }
                            write_bytes!(stdout, "\n")?;
                        }
                    }
                }
                Ok(())
            }
            check_library(stdout, ld_paths, &cxxstdlibs, cxxstdlib_paths)?;
            check_library(stdout, ld_paths, "libz.a", zlib_paths)?;
            check_library(stdout, ld_paths, "libzstd.a", zstd_paths)?;
        }

        "static"
    } else {
        "dylib"
    };

    for cxxstdlib in cxxstdlibs.iter() {
        write_bytes!(
            stdout,
            "cargo:rustc-link-lib=",
            linkage,
            "=",
            cxxstdlib,
            "\n"
        )?;
    }
    if needs_zlib {
        write_bytes!(stdout, "cargo:rustc-link-lib=", linkage, "=z\n")?;
    }
    if needs_zstd {
        write_bytes!(stdout, "cargo:rustc-link-lib=", linkage, "=zstd\n")?;
    }

    Ok(())
}

/// Points cargo to shared library name of LLVM.
///
/// Unlike [`link_llvm_static`], it does not require explicit search for
/// dependencies, since shared libraries contain `DT_NEEDED` entries that
/// specify the names of libaries that the dynamic linker should link
/// beforehand.
fn link_llvm_dynamic(stdout: &mut io::StdoutLock<'_>, llvm_lib_dir: &Path) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    const DYLIB_EXT: &[u8] = b".dylib";
    #[cfg(not(target_os = "macos"))]
    const DYLIB_EXT: &[u8] = b".so";

    let dir_entries = fs::read_dir(llvm_lib_dir).with_context(|| {
        format!(
            "failed to read entry of the directory {}",
            llvm_lib_dir.display()
        )
    })?;
    let mut found = false;
    for entry in dir_entries {
        let entry = entry.with_context(|| {
            format!(
                "failed to read entry of the directory {}",
                llvm_lib_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let file_name = file_name.as_bytes();
        if let Some(trimmed) = file_name
            .strip_prefix(b"libLLVM")
            .and_then(|name| name.strip_suffix(DYLIB_EXT))
        {
            write_bytes!(stdout, "cargo:rustc-link-lib=dylib=LLVM", trimmed, "\n")?;
            found = true;
            break;
        };
    }
    if !found {
        anyhow::bail!(
            "could not find dynamic libLLVM in the directory {}",
            llvm_lib_dir.display()
        );
    }

    Ok(())
}

/// Points cargo to the path containing LLVM libraries and to the LLVM library
/// files.
fn main() -> anyhow::Result<()> {
    let link_fn = if cfg!(feature = "no-llvm-linking") {
        if cfg!(any(
            feature = "llvm-link-static",
            feature = "llvm-deps-link-static"
        )) {
            anyhow::bail!(
                "`no-llvm-linking` and linking features (`llvm-deps-link-static`
and `llvm-link-static`) are mutually exclusive"
            );
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
        "\n",
    )?;

    link_fn(&mut stdout, &llvm_lib_dir)
}
