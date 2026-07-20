use {
  proc_macro::TokenStream,
  quote::quote,
  syn::{Data, DeriveInput, Field, Fields, GenericArgument, PathArguments, Type, parse_macro_input}
};

/// Derives the `Prompt` trait for a struct with named fields, turning each field into a CLI prompt
/// automatically. Adding / removing fields on the struct keeps the generated wizard in sync.
///
/// Field semantics :
///
/// - A required field (`T`) is always prompted for when constructing from scratch, and recursively
///   filled when partially present.
///
/// - An `Option<T>` field is prompted for optionally : the user may skip it. A value already
///   present is kept as is.
///
/// - An `Option<T>` field marked with `#[prompt(required)]` is only optional so that
///   deserialization tolerates its absence : the wizard treats it as required.
///
/// - Nested structs deriving `Prompt` recurse, forming a dot separated path out of the camelCased
///   field names (for example `repositories.sshAccess.knownHosts`), matching the YAML keys
///   produced by `#[serde(rename_all = "camelCase")]`.
#[proc_macro_derive(Prompt, attributes(prompt))]
pub fn derive_prompt(input: TokenStream) -> TokenStream {
  let input = parse_macro_input!(input as DeriveInput);
  let struct_name = &input.ident;

  let Data::Struct(r#struct) = &input.data else {
    return syn::Error::new_spanned(struct_name, "`Prompt` can only be derived for structs")
             .to_compile_error()
             .into();
  };
  let Fields::Named(fields) = &r#struct.fields else {
    return syn::Error::new_spanned(struct_name,
                                   "`Prompt` can only be derived for structs with named fields")
             .to_compile_error()
             .into();
  };

  // Field initializers for constructing the struct from scratch (`prompt`), and for completing a
  // partially present one (`prompt_or_keep`).
  let mut fresh_field_initializers = Vec::new();
  let mut fill_field_initializers = Vec::new();

  for field in &fields.named {
    let field_name = field.ident.as_ref().unwrap();
    let field_path = camel_case(&field_name.to_string());

    match (option_inner_type(&field.ty), is_marked_required(field)) {
      // An `Option<T>` field which is only optional so that deserialization tolerates its
      // absence : the wizard treats it as required.
      | (Some(inner_type), true) => {
        fresh_field_initializers.push(quote! {
          #field_name: Some(<#inner_type as ::prompt::Prompt>::prompt(prompter, &::prompt::child_path(path, #field_path))?)
        });
        fill_field_initializers.push(quote! {
          #field_name: Some(<#inner_type as ::prompt::Prompt>::prompt_or_keep(existing.#field_name, prompter, &::prompt::child_path(path, #field_path))?)
        });
      },

      // A genuinely optional field : the user may skip it. A value already present is kept as is.
      | (Some(inner_type), false) => {
        fresh_field_initializers.push(quote! {
          #field_name: <#inner_type as ::prompt::Prompt>::prompt_optional(prompter, &::prompt::child_path(path, #field_path))?
        });
        fill_field_initializers.push(quote! {
          #field_name: match existing.#field_name {
            | Some(value) => Some(value),

            | None => <#inner_type as ::prompt::Prompt>::prompt_optional(prompter, &::prompt::child_path(path, #field_path))?
          }
        });
      },

      // A required field : prompted for from scratch, recursively filled when already present.
      | (None, _) => {
        let field_type = &field.ty;

        fresh_field_initializers.push(quote! {
          #field_name: <#field_type as ::prompt::Prompt>::prompt(prompter, &::prompt::child_path(path, #field_path))?
        });
        fill_field_initializers.push(quote! {
          #field_name: <#field_type as ::prompt::Prompt>::prompt_or_keep(Some(existing.#field_name), prompter, &::prompt::child_path(path, #field_path))?
        });
      }
    }
  }

  quote! {
    impl ::prompt::Prompt for #struct_name {
      fn prompt(prompter: &mut dyn ::prompt::Prompter,
                path: &str)
                -> Result<Self, ::prompt::BoxedError> {
        Ok(Self { #(#fresh_field_initializers),* })
      }

      fn prompt_optional(prompter: &mut dyn ::prompt::Prompter,
                         path: &str)
                         -> Result<Option<Self>, ::prompt::BoxedError> {
        if prompter.confirm(&format!("Configure {path}?"))? {
          Ok(Some(<Self as ::prompt::Prompt>::prompt(prompter, path)?))
        } else {
          Ok(None)
        }
      }

      fn prompt_or_keep(existing: Option<Self>,
                        prompter: &mut dyn ::prompt::Prompter,
                        path: &str)
                        -> Result<Self, ::prompt::BoxedError> {
        match existing {
          | None => <Self as ::prompt::Prompt>::prompt(prompter, path),

          | Some(existing) => Ok(Self { #(#fill_field_initializers),* })
        }
      }
    }
  }.into()
}

/// Returns the `T` out of an `Option<T>` type, or `None` when the type isn't an `Option`.
fn option_inner_type(r#type: &Type) -> Option<&Type> {
  let Type::Path(type_path) = r#type else { return None };

  let last_segment = type_path.path.segments.last()?;
  if last_segment.ident != "Option" {
    return None;
  }

  let PathArguments::AngleBracketed(generic_arguments) = &last_segment.arguments else {
    return None;
  };
  match generic_arguments.args.first()? {
    | GenericArgument::Type(inner_type) => Some(inner_type),

    | _ => None
  }
}

/// Whether the field carries the `#[prompt(required)]` attribute.
fn is_marked_required(field: &Field) -> bool {
  field.attrs.iter().any(|attribute| {
    if !attribute.path().is_ident("prompt") {
      return false;
    }

    let mut required = false;
    let _ = attribute.parse_nested_meta(|meta| {
              if meta.path.is_ident("required") {
                required = true;
              }
              Ok(())
            });
    required
  })
}

/// Converts a snake_cased field name to its camelCased form, matching the YAML keys produced by
/// `#[serde(rename_all = "camelCase")]`.
fn camel_case(snake_cased: &str) -> String {
  let mut parts = snake_cased.split('_');

  let mut camel_cased = parts.next().unwrap_or_default().to_string();
  for part in parts {
    let mut characters = part.chars();
    if let Some(first_character) = characters.next() {
      camel_cased.push(first_character.to_ascii_uppercase());
      camel_cased.extend(characters);
    }
  }

  camel_cased
}
