use proc_macro::TokenStream;

mod modpack_setting_derive;
mod setting_derive;

#[proc_macro_derive(Setting)]
pub fn setting(input: TokenStream) -> TokenStream {
    setting_derive::setting(input)
}

#[proc_macro_derive(ModpackSetting)]
pub fn modpack_setting(input: TokenStream) -> TokenStream {
    modpack_setting_derive::modpack_setting(input)
}

#[cfg(test)]
mod tests {
    use super::*;
}
