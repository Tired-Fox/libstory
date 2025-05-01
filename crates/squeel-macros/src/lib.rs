extern crate proc_macro;

use std::{iter::Peekable, ops::AddAssign, str::{Chars, FromStr}};

use proc_macro_error::{abort, emit_error};
use quote::{quote, ToTokens, TokenStreamExt};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use syn::{parse::Parse, parse_macro_input, spanned::Spanned, AngleBracketedGenericArguments, Attribute, Data, DataStruct, DeriveInput, Expr, ExprCall, ExprLit, Fields, FieldsNamed, GenericArgument, Ident, Lit, LitInt, Meta, PathArguments, Token, Type};

#[derive(Clone, Copy, Debug)]
enum Rename {
    Snake,
    UpperSnake,
    Kebab,
    UpperKebab,
    Pascal,
    Camel,
    Lower,
    Upper,
}

struct RenameIter<'a>(Peekable<Chars<'a>>);
impl Iterator for RenameIter<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        let peek = self.0.peek()?;
        let upper = peek.is_ascii_uppercase();
        let alpha = peek.is_alphabetic();

        let value = self.0.clone()
            .take_while(|c| {
                c != &' ' && c != &'-' && c != &'_' && alpha == c.is_alphabetic() && upper == c.is_ascii_uppercase()
            })
            .collect::<String>();

        Some(value)
    }
}

impl Rename {
    pub fn apply(&self, value: impl AsRef<str>) -> String {
        match self {
            Self::Lower => value.as_ref().to_ascii_lowercase(),
            Self::Upper => value.as_ref().to_ascii_uppercase(),
            Self::Snake => RenameIter(value.as_ref().chars().peekable())
                .map(|part| part.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::UpperSnake => RenameIter(value.as_ref().chars().peekable())
                .map(|part| part.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("_"),
            Self::Kebab => RenameIter(value.as_ref().chars().peekable())
                .map(|part| part.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("-"),
            Self::UpperKebab => RenameIter(value.as_ref().chars().peekable())
                .map(|part| part.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join(" "),
            Self::Pascal => RenameIter(value.as_ref().chars().peekable())
                .map(|part| part.chars()
                    .enumerate()
                    .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
                    .collect::<String>()
                )
                .collect::<Vec<_>>()
                .join(""),
            Self::Camel => RenameIter(value.as_ref().chars().peekable())
                .enumerate()
                .map(|(i, part)| if i == 0 {
                    part.to_ascii_lowercase()
                } else {
                    part.chars()
                        .enumerate()
                        .map(|(j, c)| if j == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(""),
        }
        // Break on: ` `, `_`, `-`, digit <-> alpha, upper <-> lower
    }
}

impl FromStr for Rename {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "snake_case" => Self::Snake,
            "UPPER_SNAKE_CASE" => Self::UpperSnake,
            "kebab-case" => Self::Kebab,
            "UPPER-KEBAB-CASE" => Self::UpperKebab,
            "PascalCase" => Self::Pascal,
            "camelCase" => Self::Camel,
            "lower" => Self::Lower,
            "UPPER" => Self::Upper,
            _ => return Err("did you mean: `snake_case`, `UPPER_SNAKE_CASE`, `kebab-case`, `UPPER-KEBAB-CASE`, `camelCase`, `PascalCase`, `lower`, `UPPER`".to_string())
        })
    }
}

fn optional(ty: &Type) -> bool {
    match ty {
        Type::Path(path) => {
            let last = path.path.segments.last().unwrap();
            if "Option" == last.ident.to_string().as_str() {
                return true;
            }
        },
        Type::Reference(r) => {
            return optional(&r.elem);
        }
        _ => {}
    }
    false
}

fn type_to_sql(ty: &Type) -> String {
    match ty {
        Type::Path(path) => {
            let last = path.path.segments.last().unwrap();
            match last.ident.to_string().as_str() {
                "Option" => {
                    if let PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) = &last.arguments {
                        if let GenericArgument::Type(t) = args.first().unwrap() {
                            let v = type_to_sql(t);
                            return v;
                        }
                    }
                },
                "String" => return "TEXT".to_string(),
                "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => return "INTEGER".to_string(),
                "f32" | "f64" => return "REAL".to_string(),
                "Vec" => {
                    if let PathArguments::AngleBracketed(AngleBracketedGenericArguments { args, .. }) = &last.arguments {
                        if let GenericArgument::Type(Type::Path(tp)) = args.first().unwrap() {
                            if tp.path.segments.last().unwrap().ident.to_string().as_str() == "u8" {
                                return "BLOB".to_string();
                            }
                        }
                    }
                },
                _ => {}
            }
        },
        Type::Array(ta) => if let Type::Path(tp) = &*ta.elem {
            if tp.path.segments.last().unwrap().ident.to_string().as_str() == "u8" {
                return "BLOB".to_string();
            }
        },
        Type::Reference(r) => {
            return type_to_sql(&r.elem);
        }
        _ => {}
    }

    "TEXT".to_string()
}

struct Parameter {
    ty: Type,
    ident: Ident,
    ignore: bool,
    optional: bool,
    name: Option<Ident>,
    rename: Option<Rename>,
    primary: bool,
    foreign: Option<(Ident, Ident)>,
    unique: bool,
}

impl Parameter {
    fn sql_name(&self) -> Ident {
        let mut name = self.name.clone().unwrap_or(self.ident.clone());
        if let Some(rename) = &self.rename {
            name = Ident::new(&rename.apply(name.to_string()), name.span());
        }
        name
    }
}

impl std::fmt::Display for Parameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ty = type_to_sql(&self.ty);
        let name = self.sql_name();

        write!(f, "{} {ty}", name)?;
        if self.primary { write!(f, " PRIMARY KEY")?; }
        if !self.primary && !self.optional { write!(f, " NOT NULL")?; }

        Ok(())
    }
}

enum FlagOrAssign {
    Flag(Ident),
    Assign(Ident, Expr),
}
impl std::fmt::Debug for FlagOrAssign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag(n) => write!(f, "Flag({n})"),
            Self::Assign(n, _) => write!(f, "Assign({n})"),
        }
    }
}

impl ToTokens for FlagOrAssign {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Flag(flag) => tokens.append_all(quote! { #flag }),
            Self::Assign(name, value) => tokens.append_all(quote! { #name=#value }),
        }
    }
}

impl Parse for FlagOrAssign {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name = input.parse::<Ident>()?;
        if input.peek(Token![=]) {
            _ = input.parse::<Token![=]>();
            let value = input.parse::<Expr>()?;
            Ok(Self::Assign(name, value))
        } else {
            Ok(Self::Flag(name))
        }
    }
}

#[derive(Default, Debug)]
struct ColumnAttributes {
    name: Option<Ident>,
    rename: Option<Rename>, 
    primary: bool,
    unique: bool,
    ignore: bool,
    foreign: Option<(Ident, Ident)>,
}

impl Parse for ColumnAttributes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut attrs = Self::default();

        let items = input.parse_terminated(FlagOrAssign::parse, Token![,])?;
        for item in items {
            match item {
                FlagOrAssign::Flag(name) => {
                    match name.to_string().as_str() {
                        "primary" => attrs.primary = true,
                        "unique" => attrs.unique = true,
                        "ignore" => attrs.ignore = true,
                        _ => {}
                    }
                },
                FlagOrAssign::Assign(name, value) => match name.to_string().as_str() {
                    "rename" => if let Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) = value {
                        match Rename::from_str(&lit.value()) {
                            Ok(rename) => attrs.rename = Some(rename),
                            Err(e) => emit_error!(lit.span(), "{}", e)
                        }
                    } else {
                        emit_error!(value.span(), "expected a string literal")
                    },
                    "name" => if let Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) = value {
                        attrs.name = Some(Ident::new(&lit.value(), lit.span()))
                    } else {
                        emit_error!(value.span(), "expected a string literal")
                    },
                    "foreign" => if let Expr::Call(ExprCall { func, args, .. }) = value {
                        let table = if let Expr::Path(path) = *func {
                            path.path.segments.last().unwrap().ident.clone()
                        } else {
                            abort!(func.span(), "expected a table indentifier");
                        };

                        if args.len() != 1 { abort!(args.span(), "expected only 1 column") }

                        let param = if let Expr::Path(path) = args.first().unwrap() {
                            path.path.segments.last().unwrap().ident.clone()
                        } else {
                            abort!(args.first().unwrap().span(), "expected a table column identifier");
                        };

                        attrs.foreign = Some((table, param));
                    } else {
                        emit_error!(value.span(), "expected the syntax `Table(column)`")
                    },
                    _ => {}
                }
            }
        }

        Ok(attrs)
    }
}

impl AddAssign for ColumnAttributes {
    fn add_assign(&mut self, rhs: Self) {
        self.primary = self.primary || rhs.primary;
        self.unique = self.unique || rhs.unique;
        self.name = rhs.name.or(self.name.clone());
        self.rename = rhs.rename.or(self.rename);
        self.foreign = rhs.foreign.or(self.foreign.clone());
        self.ignore = self.ignore || rhs.ignore;
    }
}

impl AddAssign<&TableAttributes> for ColumnAttributes {
    fn add_assign(&mut self, rhs: &TableAttributes) {
        self.rename = self.rename.or(rhs.rename_all);
    }
}

impl From<(&TableAttributes, &[Attribute])> for ColumnAttributes {
    fn from((table_attrs, value): (&TableAttributes, &[Attribute])) -> Self {
        let mut attrs = Self::default();

        for attr in value {
            if let Attribute { meta: Meta::List(meta), .. } = attr {
                if "column" == meta.path.segments.last().unwrap().ident.to_string().as_str() {
                    match syn::parse::<ColumnAttributes>(meta.tokens.clone().into()) {
                        Ok(a) => attrs += a,
                        Err(e) => emit_error!(e.span(), "CATTRERR: {}", e),
                    }
                }
            }
        }

        attrs += table_attrs;

        attrs
    }
}

#[derive(Default, Debug)]
struct TableAttributes {
    name: Option<Ident>,
    rename: Option<Rename>, 
    rename_all: Option<Rename>, 
}

impl Parse for TableAttributes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut attrs = Self::default();

        let items = input.parse_terminated(FlagOrAssign::parse, Token![,])?;
        for item in items {
            match item {
                FlagOrAssign::Flag(name) => match name.to_string().as_str() {
                    _ => emit_error!(name.span(), "invalid attribute")
                }
                FlagOrAssign::Assign(name, value) => match name.to_string().as_str() {
                    "rename" => if let Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) = value {
                        match Rename::from_str(&lit.value()) {
                            Ok(rename) => attrs.rename = Some(rename),
                            Err(e) => emit_error!(lit.span(), "{}", e)
                        }
                    } else {
                        emit_error!(value.span(), "expected a string literal")
                    },
                    "rename_all" => if let Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) = value {
                        match Rename::from_str(&lit.value()) {
                            Ok(rename) => attrs.rename_all = Some(rename),
                            Err(e) => emit_error!(lit.span(), "{}", e)
                        }
                    } else {
                        emit_error!(value.span(), "expected a string literal")
                    },
                    "name" => if let Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) = value {
                        attrs.name = Some(Ident::new(&lit.value(), lit.span()))
                    } else {
                        emit_error!(value.span(), "expected a string literal")
                    },
                    _ => {}
                }
            }
        }

        Ok(attrs)
    }
}

impl AddAssign for TableAttributes {
    fn add_assign(&mut self, rhs: Self) {
        self.name = rhs.name.or(self.name.clone());
        self.rename = rhs.rename.or(self.rename);
        self.rename_all = rhs.rename_all.or(self.rename_all);
    }
}

impl From<&[Attribute]> for TableAttributes {
    fn from(value: &[Attribute]) -> Self {
        let mut attrs = Self::default();

        for attr in value {
            if let Attribute { meta: Meta::List(meta), .. } = attr {
                if "table" == meta.path.segments.last().unwrap().ident.to_string().as_str() {
                    match syn::parse::<TableAttributes>(meta.tokens.clone().into()) {
                        Ok(a) => attrs += a,
                        Err(e) => emit_error!(e.span(), "CATTRERR: {}", e),
                    }
                }
            }
        }

        attrs
    }
}

fn parse_parameters(derive: &DeriveInput, attrs: &TableAttributes) -> Vec<Parameter> {
    if let Data::Struct(DataStruct { fields: Fields::Named(FieldsNamed { named: fields, .. }), .. }) = &derive.data {
        return fields.iter()
            .map(|f| {
                let attrs = ColumnAttributes::from((attrs, f.attrs.as_ref()));

                let ident = f.ident.clone().unwrap();
                let ty = f.ty.clone();
                Parameter {
                    optional: optional(&ty),
                    ignore: attrs.ignore,
                    name: attrs.name,
                    rename: attrs.rename,
                    primary: attrs.primary,
                    unique: attrs.unique,
                    foreign: attrs.foreign,
                    ty,
                    ident,
                }
            })
            .collect::<Vec<_>>();
    }
    Vec::new()
}

#[proc_macro_error::proc_macro_error]
#[proc_macro_derive(Table, attributes(table, column))]
pub fn table_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let attrs = TableAttributes::from(input.attrs.as_ref());

    let struct_name = input.ident.clone();
    let mut table_name = attrs.name.clone().unwrap_or(input.ident.clone()).to_string();
    if let Some(rename) = attrs.rename {
        table_name = rename.apply(&table_name);
    }

    let params: Vec<Parameter> = parse_parameters(&input, &attrs);

    let mut from_row_arg_names = Vec::new();
    let mut from_row_arg_build = Vec::new();
    let mut from_row_types = Vec::new();

    let mut ignored = Vec::new();
    let mut unique = Vec::new();
    let mut foreign = Vec::new();
    let mut create_format_args = Vec::new();
    let mut column_renames = Vec::new();

    let mut new_arg = TokenStream2::new();
    let mut fetch_arg = TokenStream2::new();

    let mut returning = Vec::new();
    let mut keys_response = TokenStream2::new();

    let mut insert_fmt = Vec::new();
    let mut insert_binding = TokenStream2::new();

    let mut update_fmt = Vec::new();
    let mut update_binding = TokenStream2::new();

    let mut delete_fmt = Vec::new();
    let mut delete_binding = TokenStream2::new();

    let mut fetch_binding = TokenStream2::new();
    let mut fetch_query = TokenStream2::new();

    for (i, param) in params.iter().enumerate() {
        let idx = LitInt::new(&i.to_string(), Span::call_site());
        let ty = param.ty.clone();
        let name_ident = param.ident.clone();
        let name = param.sql_name().to_string();

        if param.ignore {
            ignored.push(name_ident);
            continue;
        }

        if !param.primary {
            let idx = LitInt::new(&insert_fmt.len().to_string(), Span::call_site());
            new_arg.append_all(quote! { #ty, });
            insert_binding.append_all(quote!{ .bind(args.#idx) });
            insert_fmt.push(name.clone());
        }

        update_fmt.push(name.clone());
        update_binding.append_all(quote!{ .bind(&self.#name_ident) });
        let fq = format!("{name} = ?");
        fetch_query.append_all(quote! { if fetch.#idx.is_some() { where_clause.push(#fq) } });
        fetch_binding.append_all(quote! { if let Some(v) = fetch.#idx { query = query.bind(v); } });
        if param.optional {
            fetch_arg.append_all(quote! { #ty, });
        } else {
            fetch_arg.append_all(quote! { Option<#ty>, });
        }

        if param.primary || param.foreign.is_some() {
            delete_fmt.push(name.clone());
            delete_binding.append_all(quote!{ .bind(&self.#name_ident) });
            keys_response.append_all(quote!{ #ty, });
            returning.push(name.clone());
        }

        if param.name.is_some() || param.rename.is_some() {
            let m = name_ident.to_string();
            column_renames.push(quote! { #m => #name })
        }

        if param.unique { unique.push(name.clone()); }
        if let Some(f) = param.foreign.as_ref() {
            foreign.push(format!("FOREIGN KEY ({}) REFERENCES {{}}({{}})", name));

            let name = f.0.clone();
            let value = f.1.to_string();
            create_format_args.push(quote! { #name::table_name(), #name::column_name(#value) })
        }

        from_row_types.push(quote!{#ty: ::sqlx::decode::Decode<'a, R::Database>,
            #ty: ::sqlx::types::Type<R::Database>,
        });
        from_row_arg_build.push(quote!(let #name_ident: #ty = __row.try_get(#name)?;));
        from_row_arg_names.push(quote!(#name_ident,));
    }

    let insert_fmt = format!("INSERT{} INTO {{}} ({}) VALUES ({}){}",
        if unique.is_empty() { "" } else { " OR IGNORE" },
        insert_fmt.join(", "),
        insert_fmt.iter().map(|_| "?").collect::<Vec<_>>().join(", "),
        if returning.is_empty() { String::new() } else { format!(" RETURNING {}", returning.join(",")) }
    );
    let update_fmt = format!("INSERT INTO {{}} ({}) VALUES ({}){}",
        update_fmt.join(", "),
        update_fmt.iter().map(|_| "?").collect::<Vec<_>>().join(", "),
        if returning.is_empty() { String::new() } else { format!(" RETURNING {}", returning.join(",")) }
    );
    let delete_fmt = format!("DELETE FROM {{}} WHERE {}{}",
        delete_fmt.iter().map(|v| format!("{v} = ?")).collect::<Vec<_>>().join(" AND "),
        if returning.is_empty() { String::new() } else { format!(" RETURNING {}", returning.join(",")) }
    );

    let create = format!(
        "CREATE TABLE IF NOT EXISTS {struct_name} ({}{}{});",
        params.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
        if unique.is_empty() { String::new() } else { format!(",UNIQUE({})", unique.join(",")) },
        if foreign.is_empty() { String::new() } else { format!(",{}", foreign.join(",")) },
    );

    quote! {
        impl squeel::Table for #struct_name {
            type New = (#new_arg);
            type Fetch = (#fetch_arg);
            type Keys = (#keys_response);

            fn table_name() -> &'static str { #table_name }

            fn column_name(name: &str) -> &str {
                match name {
                    #(#column_renames,)*
                    _ => name,
                }
            }

            fn scaffold() -> String {
                format!(
                    #create,
                    #(#create_format_args,)*
                )
            }

            async fn create(pool: &sqlx::Pool<sqlx::sqlite::Sqlite>) -> sqlx::Result<()> {
                let mut conn = pool.acquire().await?;
                sqlx::query(&Self::scaffold())
                    .execute(&mut * conn)
                    .await?;

                Ok(())
            }

            async fn insert(pool: &sqlx::Pool<sqlx::sqlite::Sqlite>, args: Self::New) -> sqlx::Result<Self::Keys> {
                let mut conn = pool.acquire().await?;
                sqlx::query_as(&format!(#insert_fmt, Self::table_name()))
                    #insert_binding
                    .fetch_one(&mut *conn)
                    .await
            }

            async fn update(&self, pool: &sqlx::Pool<sqlx::sqlite::Sqlite>) -> sqlx::Result<Self::Keys> {
                let mut conn = pool.acquire().await?;
                sqlx::query_as(&format!(#update_fmt, Self::table_name()))
                    #update_binding
                    .fetch_one(&mut *conn)
                    .await
            }

            async fn delete(&self, pool: &sqlx::Pool<sqlx::sqlite::Sqlite>) -> sqlx::Result<Self::Keys> {
                let mut conn = pool.acquire().await?;
                sqlx::query_as(&format!(#delete_fmt, Self::table_name()))
                    #delete_binding
                    .fetch_one(&mut *conn)
                    .await
            }

            async fn one(pool: &sqlx::Pool<sqlx::sqlite::Sqlite>, fetch: Self::Fetch) -> sqlx::Result<Self> {
                let mut conn = pool.acquire().await?;

                let mut where_clause = Vec::new();
                #fetch_query
                let mut q = format!(
                    "SELECT * FROM {}{}",
                    Self::table_name(),
                    if where_clause.is_empty() { String::new() }
                    else { format!(" WHERE {}", where_clause.join(" AND ")) },
                );

                let mut query = sqlx::query_as(&q);
                #fetch_binding

                query.fetch_one(&mut *conn).await
            }

            async fn many(pool: &sqlx::Pool<sqlx::sqlite::Sqlite>, fetch: Self::Fetch) -> sqlx::Result<Vec<Self>> {
                let mut conn = pool.acquire().await?;

                let mut where_clause = Vec::new();
                #fetch_query
                let mut q = format!(
                    "SELECT * FROM {}{}",
                    Self::table_name(),
                    if where_clause.is_empty() { String::new() }
                    else { format!(" WHERE {}", where_clause.join(" AND ")) },
                );

                let mut query = sqlx::query_as(&q);
                #fetch_binding

                query.fetch_all(&mut *conn).await
            }
        }

        impl<'a, R: sqlx::Row> ::sqlx::FromRow<'a, R> for #struct_name
        where
            &'a ::std::primitive::str: ::sqlx::ColumnIndex<R>,
            #(#from_row_types)*
        {
            fn from_row(__row: &'a R) -> ::sqlx::Result<Self> {
                #(#from_row_arg_build)*
                ::std::result::Result::Ok(#struct_name {
                    #(#from_row_arg_names)*
                    #(#ignored: Default::default(),)*
                })
            }
        }
    }.into()
}
