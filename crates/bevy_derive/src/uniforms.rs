use crate::modules::{get_modules, get_path};
use darling::FromMeta;
use inflector::Inflector;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DataStruct, DeriveInput, Field, Fields, Path};

#[derive(FromMeta, Debug, Default)]
struct UniformAttributeArgs {
    #[darling(default)]
    pub ignore: Option<bool>,
    #[darling(default)]
    pub shader_def: Option<bool>,
    #[darling(default)]
    pub instance: Option<bool>,
    #[darling(default)]
    pub vertex: Option<bool>,
    #[darling(default)]
    pub buffer: Option<bool>,
}

#[derive(Default)]
struct UniformAttributes {
    pub ignore: bool,
    pub shader_def: bool,
    pub instance: bool,
    pub vertex: bool,
    pub buffer: bool,
}

impl From<UniformAttributeArgs> for UniformAttributes {
    fn from(args: UniformAttributeArgs) -> Self {
        UniformAttributes {
            ignore: args.ignore.unwrap_or(false),
            shader_def: args.shader_def.unwrap_or(false),
            instance: args.instance.unwrap_or(false),
            vertex: args.vertex.unwrap_or(false),
            buffer: args.buffer.unwrap_or(false),
        }
    }
}

static UNIFORM_ATTRIBUTE_NAME: &'static str = "uniform";

pub fn derive_uniforms(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    let modules = get_modules(&ast);

    let bevy_render_path: Path = get_path(&modules.bevy_render);
    let bevy_core_path: Path = get_path(&modules.bevy_core);
    let bevy_asset_path: Path = get_path(&modules.bevy_asset);

    let fields = match &ast.data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => &fields.named,
        _ => panic!("expected a struct with named fields"),
    };

    let field_attributes = fields
        .iter()
        .map(|f| {
            (
                f,
                f.attrs
                    .iter()
                    .find(|a| {
                        a.path.get_ident().as_ref().unwrap().to_string() == UNIFORM_ATTRIBUTE_NAME
                    })
                    .map(|a| {
                        UniformAttributes::from(
                            UniformAttributeArgs::from_meta(&a.parse_meta().unwrap())
                                .unwrap_or_else(|_err| UniformAttributeArgs::default()),
                        )
                    })
                    .unwrap_or_else(|| UniformAttributes::default()),
            )
        })
        .collect::<Vec<(&Field, UniformAttributes)>>();

    let struct_name = &ast.ident;

    let mut active_uniform_field_names = Vec::new();
    let mut active_uniform_field_name_strings = Vec::new();
    let mut uniform_name_strings = Vec::new();
    let mut texture_and_sampler_name_strings = Vec::new();
    let mut texture_and_sampler_name_idents = Vec::new();
    let mut field_infos = Vec::new();
    let mut get_field_bind_types = Vec::new();

    let mut vertex_buffer_field_names_pascal = Vec::new();
    let mut vertex_buffer_field_types = Vec::new();

    let mut shader_def_field_names = Vec::new();
    let mut shader_def_field_names_screaming_snake = Vec::new();

    for (f, attrs) in field_attributes.iter() {
        let field_name = f.ident.as_ref().unwrap().to_string();
        if !attrs.ignore {
            let active_uniform_field_name = &f.ident;
            active_uniform_field_names.push(&f.ident);
            active_uniform_field_name_strings.push(field_name.clone());
            let uniform = format!("{}_{}", struct_name, field_name);
            let texture = format!("{}", uniform);
            let sampler = format!("{}_sampler", uniform);
            uniform_name_strings.push(uniform.clone());
            texture_and_sampler_name_strings.push(texture.clone());
            texture_and_sampler_name_strings.push(sampler.clone());
            texture_and_sampler_name_idents.push(f.ident.clone());
            texture_and_sampler_name_idents.push(f.ident.clone());
            let is_instanceable = attrs.instance;
            field_infos.push(quote!(#bevy_render_path::shader::FieldInfo {
                name: #field_name,
                uniform_name: #uniform,
                texture_name: #texture,
                sampler_name: #sampler,
                is_instanceable: #is_instanceable,
            }));

            if attrs.buffer {
                get_field_bind_types.push(quote!({
                    let bind_type = self.#active_uniform_field_name.get_bind_type();
                    let size = if let Some(#bevy_render_path::shader::FieldBindType::Uniform { size }) = bind_type {
                        size
                    } else {
                        panic!("Uniform field was labeled as a 'buffer', but it does not have a compatible type.")
                    };
                    Some(#bevy_render_path::shader::FieldBindType::Buffer { size })
                }))
            } else {
                get_field_bind_types.push(quote!(self.#active_uniform_field_name.get_bind_type()))
            }
        }

        if attrs.shader_def {
            shader_def_field_names.push(&f.ident);
            shader_def_field_names_screaming_snake.push(field_name.to_screaming_snake_case())
        }

        if attrs.instance || attrs.vertex {
            vertex_buffer_field_types.push(&f.ty);
            let pascal_field = f.ident.as_ref().unwrap().to_string().to_pascal_case();
            vertex_buffer_field_names_pascal.push(if attrs.instance {
                format!("I_{}_{}", struct_name, pascal_field)
            } else {
                format!("{}_{}", struct_name, pascal_field)
            });
        }
    }

    let struct_name_string = struct_name.to_string();
    let struct_name_uppercase = struct_name_string.to_uppercase();
    let field_infos_ident = format_ident!("{}_FIELD_INFO", struct_name_uppercase);
    let vertex_buffer_descriptor_ident =
        format_ident!("{}_VERTEX_BUFFER_DESCRIPTOR", struct_name_uppercase);

    TokenStream::from(quote! {
        static #field_infos_ident: &[#bevy_render_path::shader::FieldInfo] = &[
            #(#field_infos,)*
        ];

        static #vertex_buffer_descriptor_ident: #bevy_render_path::once_cell::sync::Lazy<#bevy_render_path::pipeline::VertexBufferDescriptor> =
            #bevy_render_path::once_cell::sync::Lazy::new(|| {
                use #bevy_render_path::pipeline::{VertexFormat, AsVertexFormats, VertexAttributeDescriptor};

                let mut vertex_formats: Vec<(&str,&[VertexFormat])>  = vec![
                    #((#vertex_buffer_field_names_pascal, <#vertex_buffer_field_types>::as_vertex_formats()),)*
                ];

                let mut shader_location = 0;
                let mut offset = 0;
                let vertex_attribute_descriptors = vertex_formats.drain(..).map(|(name, formats)| {
                    formats.iter().enumerate().map(|(i, format)| {
                        let size = format.get_size();
                        let formatted_name = if formats.len() > 1 {
                            format!("{}_{}", name, i)
                        } else {
                            format!("{}", name)
                        };
                        let descriptor = VertexAttributeDescriptor {
                            name: formatted_name.into(),
                            offset,
                            format: *format,
                            shader_location,
                        };
                        offset += size;
                        shader_location += 1;
                        descriptor
                    }).collect::<Vec<VertexAttributeDescriptor>>()
                }).flatten().collect::<Vec<VertexAttributeDescriptor>>();

                #bevy_render_path::pipeline::VertexBufferDescriptor {
                    attributes: vertex_attribute_descriptors,
                    name: #struct_name_string.into(),
                    step_mode: #bevy_render_path::pipeline::InputStepMode::Instance,
                    stride: offset,
                }
            });

        impl #bevy_render_path::shader::AsUniforms for #struct_name {
            fn get_field_infos() -> &'static [#bevy_render_path::shader::FieldInfo] {
                #field_infos_ident
            }

            fn get_field_bind_type(&self, name: &str) -> Option<#bevy_render_path::shader::FieldBindType> {
                use #bevy_render_path::shader::GetFieldBindType;
                match name {
                    #(#active_uniform_field_name_strings => #get_field_bind_types,)*
                    _ => None,
                }
            }

            fn get_uniform_texture(&self, name: &str) -> Option<#bevy_asset_path::Handle<#bevy_render_path::texture::Texture>> {
                use #bevy_render_path::shader::GetTexture;
                match name {
                    #(#texture_and_sampler_name_strings => self.#texture_and_sampler_name_idents.get_texture(),)*
                    _ => None,
                }
            }

            fn write_uniform_bytes(&self, name: &str, buffer: &mut [u8]) {
                use #bevy_core_path::bytes::Bytes;
                match name {
                    #(#uniform_name_strings => self.#active_uniform_field_names.write_bytes(buffer),)*
                    _ => {},
                }
            }
            fn uniform_byte_len(&self, name: &str) -> usize {
                use #bevy_core_path::bytes::Bytes;
                match name {
                    #(#uniform_name_strings => self.#active_uniform_field_names.byte_len(),)*
                    _ => 0,
                }
            }

            // TODO: move this to field_info and add has_shader_def(&self, &str) -> bool
            // TODO: this will be very allocation heavy. find a way to either make this allocation free
            // or alternatively only run it when the shader_defs have changed
            fn get_shader_defs(&self) -> Option<Vec<String>> {
                use #bevy_render_path::shader::ShaderDefSuffixProvider;
                let mut potential_shader_defs: Vec<(&'static str, Option<&'static str>)> = vec![
                    #((#shader_def_field_names_screaming_snake, self.#shader_def_field_names.get_shader_def()),)*
                ];

                Some(potential_shader_defs.drain(..)
                    .filter(|(f, shader_def)| shader_def.is_some())
                    .map(|(f, shader_def)| format!("{}_{}{}", #struct_name_uppercase, f, shader_def.unwrap()))
                    .collect::<Vec<String>>())
            }

            fn get_vertex_buffer_descriptor() -> Option<&'static #bevy_render_path::pipeline::VertexBufferDescriptor> {
                if #vertex_buffer_descriptor_ident.attributes.len() == 0 {
                    None
                } else {
                    Some(&#vertex_buffer_descriptor_ident)
                }
            }
        }
    })
}

pub fn derive_uniform(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let modules = get_modules(&ast);
    let bevy_asset_path = get_path(&modules.bevy_asset);
    let bevy_core_path = get_path(&modules.bevy_core);
    let bevy_render_path = get_path(&modules.bevy_render);

    let generics = ast.generics;
    let (impl_generics, ty_generics, _where_clause) = generics.split_for_impl();

    let struct_name = &ast.ident;
    let struct_name_string = struct_name.to_string();

    TokenStream::from(quote! {
        impl #impl_generics #bevy_render_path::shader::AsUniforms for #struct_name#ty_generics {
            fn get_field_infos() -> &'static [#bevy_render_path::shader::FieldInfo] {
                static FIELD_INFOS: &[#bevy_render_path::shader::FieldInfo] = &[
                    #bevy_render_path::shader::FieldInfo {
                       name: #struct_name_string,
                       uniform_name: #struct_name_string,
                       texture_name: #struct_name_string,
                       sampler_name: #struct_name_string,
                       is_instanceable: false,
                   }
                ];
                &FIELD_INFOS
            }

            fn get_field_bind_type(&self, name: &str) -> Option<#bevy_render_path::shader::FieldBindType> {
                use #bevy_render_path::shader::GetFieldBindType;
                match name {
                    #struct_name_string => self.get_bind_type(),
                    _ => None,
                }
            }

            fn write_uniform_bytes(&self, name: &str, buffer: &mut [u8]) {
                use #bevy_core_path::bytes::Bytes;
                match name {
                    #struct_name_string => self.write_bytes(buffer),
                    _ => {},
                }
            }
            fn uniform_byte_len(&self, name: &str) -> usize {
                use #bevy_core_path::bytes::Bytes;
                match name {
                    #struct_name_string => self.byte_len(),
                    _ => 0,
                }
            }

            fn get_uniform_texture(&self, name: &str) -> Option<#bevy_asset_path::Handle<#bevy_render_path::texture::Texture>> {
                None
            }

            // TODO: move this to field_info and add has_shader_def(&self, &str) -> bool
            // TODO: this will be very allocation heavy. find a way to either make this allocation free
            // or alternatively only run it when the shader_defs have changed
            fn get_shader_defs(&self) -> Option<Vec<String>> {
                None
            }

            fn get_vertex_buffer_descriptor() -> Option<&'static #bevy_render_path::pipeline::VertexBufferDescriptor> {
                None
            }
        }
    })
}