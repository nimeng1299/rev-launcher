use proc_macro::TokenStream;

pub fn modpack_setting(input: TokenStream) -> TokenStream {
    let st = syn::parse_macro_input!(input as syn::DeriveInput);
    match expand(&st) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
fn expand(st: &syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let struct_name = &st.ident;
    let struct_fields = get_fields(st)?;
    let fields_name_vec: Vec<_> = struct_fields.iter().map(|f| &f.ident).collect();
    let fields_type_vec: Vec<_> = struct_fields.iter().map(|f| get_option_fields(&f.ty)).collect();
    let rec = quote::quote! {
        impl #struct_name{
            pub fn read(json: serde_json::Value) -> anyhow::Result<Self>{
                #(let #fields_name_vec = #fields_type_vec::read_modpack(json.get(stringify!(#fields_name_vec)).cloned())?;)*

                Ok(Self {
                    #(#fields_name_vec,)*
                })


            }
            pub fn create() -> anyhow::Result<Self> {
                Ok(Self {
                    #(#fields_name_vec: None,)*
                })
            }
            pub fn save(&self) -> anyhow::Result<serde_json::Value> {
                let mut json_data: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
                #(
                    if let Some(ref item) = self.#fields_name_vec {
                        json_data.insert(stringify!(#fields_name_vec).to_string(), item.write()?);
                    }
                )*
                let result: serde_json::Value = serde_json::Value::Object(json_data);
                Ok(result)
            }
            pub fn get(&self, item_name: String, globle: &Settings) -> anyhow::Result<serde_json::Value> {
                match item_name.as_str() {
                    #(stringify!(#fields_name_vec) => match &self.#fields_name_vec{
                        Some(v) => v.send(),
                        None => globle.get(item_name),
                    },)*
                    _ => Err(anyhow::anyhow!("Item not found")),
                }
            }
            pub fn change(&mut self, item_name: String, value: Vec<String>, globle: &Settings) -> anyhow::Result<()> {
                match item_name.as_str() {
                    #(stringify!(#fields_name_vec) => match &mut self.#fields_name_vec{
                        Some(v) => v.receive(value),
                        None => {
                            let mut item_value = globle.get(item_name)?;
                            let mut item = #fields_type_vec::read(Some(item_value))?;
                            item.receive(value)?;
                            self.#fields_name_vec = Some(item);
                            Ok(())
                        },
                    },)*
                    _ => Err(anyhow::anyhow!("Item not found")),
                }
            }
        }
    };
    syn::Result::Ok(rec)
}

fn get_fields(
    st: &syn::DeriveInput,
) -> syn::Result<&syn::punctuated::Punctuated<syn::Field, syn::Token![,]>> {
    if let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(syn::FieldsNamed { ref named, .. }),
        ..
    }) = st.data
    {
        syn::Result::Ok(named)
    } else {
        syn::Result::Err(syn::Error::new_spanned(
            st,
            "Setting derive macro can only be used on structs with named fields",
        ))
    }
}

fn get_option_fields(ty: &syn::Type ) -> &syn::Type{
    if let syn::Type::Path(syn::TypePath { path: syn::Path{segments, ..}, .. }) = ty {
        if let Some(seg) = segments.last() {
            if seg.ident == "Option" {
                if let syn::PathArguments::AngleBracketed(syn::AngleBracketedGenericArguments{args,..}) = &seg.arguments {
                    if let Some(syn::GenericArgument::Type(inner_type)) = args.first() {
                        return inner_type;
                    }
                }
            }
        }
    }
    panic!("modpack_setting derive macro can only be used on all Option<T> field")
}
