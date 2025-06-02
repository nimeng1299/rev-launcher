use proc_macro::TokenStream;

mod setting_derive;

#[proc_macro_derive(Setting)]
pub fn setting(input: TokenStream) -> TokenStream {
    setting_derive::setting(input)
}

#[cfg(test)]
mod tests {
    use super::*;
}
