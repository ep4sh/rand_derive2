use proc_macro2::{Ident, TokenStream};
use quote::{quote, ToTokens};
use syn::{Data, DeriveInput, Field, FieldsNamed, FieldsUnnamed, Type, TypePath};

use crate::parser::{attrs_to_customizes, fixed_value, has_customize, Customize};
use quote::format_ident;
use std::collections::HashMap;

const TRAIT_NAME: &str = "TestDataProvider";

pub type TraitMethods = HashMap<String, TokenStream>;

pub(crate) fn transform(input: DeriveInput) -> TokenStream {
    let name = &input.ident;

    let mut trait_methods = TraitMethods::new();

    let ts = match input.data {
        Data::Struct(ds) => crate::gen_struct::generate(name, &mut trait_methods, ds),
        Data::Enum(de) => crate::gen_enum::generate(name, &mut trait_methods, de),
        Data::Union(_) => panic!("Unions are currently not supported"),
    };

    let mut tokens = TokenStream::new();

    if !trait_methods.is_empty() {
        let trait_methods = trait_methods.values().cloned().collect::<Vec<_>>();
        let trait_name = trait_name(name);

        tokens.extend(quote! {
            pub trait #trait_name {
                #(#trait_methods)*
            }
        })
    }

    tokens.extend(quote! {
        // Set the attribute unreachable code here, since there is a field attribute 'panic' in which
        // the type can not be generated
        #[allow(unreachable_code)]
        impl rand::distributions::Distribution<#name> for rand::distributions::Standard {
            fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> #name {
                use rand::Rng;

                #ts
            }
        }

        impl #name {
            pub fn generate_random() -> Self {
                rand::random()
            }

            pub fn generate_random_customize<T: FnOnce(&mut Self)>(customize: T) -> Self {
                let mut entity = rand::random();

                customize(&mut entity);

                entity
            }
        }
    });

    tokens
}

fn trait_name(name: &Ident) -> Ident {
    format_ident!("{}For{}", TRAIT_NAME, name)
}

fn extract_type(t: &Type) -> (String, String) {
    match t {
        Type::Path(tp) => extract_type_path(tp),
        Type::Reference(r) => extract_type(&r.elem),
        _ => panic!("This type is not supported: {:#?}", t),
    }
}

// TODO: This should actually be called recursively for when e.g. a vec in a vec must be generated
fn generated_values(
    type_ident: &Ident,
    field_ident: Option<Ident>,
    field: Field,
    trait_methods: &mut TraitMethods,
) -> TokenStream {
    let customizes = attrs_to_customizes(&field.attrs);
    let ty = field.ty;
    let prefix = match &field_ident {
        None => quote! {},
        Some(i) => quote! {
            #i:
        },
    };

    let (full_type, to_string) = extract_type(&ty);
    let ts_value = generate_value(&to_string, &customizes);
    let value = if has_customize(&customizes, Customize::Panic) {
        quote! {
            panic!("This property can not be generated")
        }
    } else if has_customize(&customizes, Customize::Custom) {
        add_to_trait_methods(type_ident, &field_ident, &ty, &to_string, trait_methods)
    } else if to_string == "Option" {
        // TODO: nicer way to get the inner type?
        let inner =
            &full_type[full_type.find("Option<").unwrap() + 7..full_type.rfind('>').unwrap()];
        let ts_value = generate_value(inner, &customizes);

        if has_customize(&customizes, Customize::AlwaysNone) {
            quote! {
                None
            }
        } else if has_customize(&customizes, Customize::AlwaysSome) {
            quote! {
                Some(#ts_value)
            }
        } else {
            quote! {
                if rng.gen() {
                    Some(#ts_value)
                } else {
                    None
                }
            }
        }
    } else if to_string == "Vec" {
        if has_customize(&customizes, Customize::Empty) {
            quote! {
                vec![]
            }
        } else {
            // TODO: recursion?
            quote! {
                vec![#ts_value]
            }
        }
    } else {
        ts_value
    };

    quote! {
        #prefix #value
    }
}

fn extract_type_path(tp: &TypePath) -> (String, String) {
    let full_type = tp
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<String>();
    let to_string = &tp.path.segments.last().unwrap().ident.to_string();

    (full_type, to_string.to_string())
}

fn add_to_trait_methods(
    type_ident: &Ident,
    field_ident: &Option<Ident>,
    ty: &Type,
    ty_str: &str,
    trait_methods: &mut TraitMethods,
) -> TokenStream {
    let trait_name = trait_name(type_ident);
    let generate_ty_name = match field_ident {
        None => format_ident!("generate_random_{}", ty_str.to_lowercase()),
        Some(f) => format_ident!("generate_{}", f),
    };

    trait_methods.insert(
        generate_ty_name.to_string(),
        quote! {
           fn #generate_ty_name<R: rand::Rng + ?Sized>(rng: &mut R) -> #ty;
        },
    );

    quote! {
        <#type_ident as #trait_name>::#generate_ty_name(rng)
    }
}

fn generate_value(ty_str: &str, customizes: &[Customize]) -> TokenStream {
    let fixed_value = fixed_value(customizes);

    if ty_str == "String" || ty_str == "str" {
        if let Some(fixed_value) = fixed_value {
            return if ty_str == "String" {
                quote! {
                    stringify!(#fixed_value).to_string()
                }
            } else {
                quote! {
                    stringify!(#fixed_value)
                }
            };
        }
    }

    if let Some(fixed_value) = fixed_value {
        return quote! {
            #fixed_value
        };
    }

    if has_customize(customizes, Customize::Default) {
        quote! {
            Default::default()
        }
    } else if ty_str == "String" {
        quote! {
            rng
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(10)
                .map(char::from)
                .collect()
        }
    } else if ty_str == "Uuid" {
        quote! {
            uuid::Uuid::new_v4()
        }
    } else {
        quote! {
            rng.gen()
        }
    }
}

pub fn generated_values_for_unnamed_fields(
    type_ident: &Ident,
    unnamed: FieldsUnnamed,
    map: &mut TraitMethods,
) -> Vec<TokenStream> {
    unnamed
        .unnamed
        .into_iter()
        .map(|r| generated_values(type_ident, None, r, map))
        .collect()
}

pub fn generated_values_for_named_fields(
    type_ident: &Ident,
    named: FieldsNamed,
    map: &mut TraitMethods,
) -> Vec<TokenStream> {
    named
        .named
        .into_iter()
        .map(|r| generated_values(type_ident, r.ident.clone(), r, map))
        .collect()
}
