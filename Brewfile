# The following packages provide libraries needed to link libLLVM and LLVM
# tools used by the compiletests.

# We use it as a linker during LLVM builds (as it's significantly faster than
# Apple ld64) and it's not available on macOS runners by default.
brew "lld"
# Provides static libc++ and FileCheck.
brew "llvm"
brew "zlib"
brew "zstd"
