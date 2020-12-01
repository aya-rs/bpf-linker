// no-prefer-dynamic

#![crate_type = "proc-macro"]

extern crate proc_macro;

use proc_macro::{TokenStream};

#[proc_macro]
pub fn macro_test(input_stream: TokenStream) -> TokenStream {
    TokenStream::new()
}
