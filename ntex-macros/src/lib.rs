#![recursion_limit = "512"]
//! web macros module
//!
//! Generators for routes
//!
//! ## Route
//!
//! Macros:
//!
//! - [get](attr.web_get.html)
//! - [post](attr.web_post.html)
//! - [put](attr.web_put.html)
//! - [delete](attr.web_delete.html)
//! - [head](attr.web_head.html)
//! - [connect](attr.web_connect.html)
//! - [options](attr.web_options.html)
//! - [trace](attr.web_trace.html)
//! - [patch](attr.web_patch.html)
//!
//! ### Attributes:
//!
//! - `"path"` - Raw literal string with path for which to register handle. Mandatory.
//! - `guard = "function_name"` - Registers function as guard using `ntex::web::guard::fn_guard`
//! - `error = "ErrorRenderer"` - Register handler for specified error renderer
//!
//! ## Notes
//!
//! Function name can be specified as any expression that is going to be accessible to the generate
//! code (e.g `my_guard` or `my_module::my_guard`)
//!
//! ## Example:
//!
//! ```rust
//! use ntex::web::{get, Error, HttpResponse};
//! use futures::{future, Future};
//!
//! #[get("/test")]
//! async fn async_test() -> Result<HttpResponse, Error> {
//!     Ok(HttpResponse::Ok().finish())
//! }
//! ```

extern crate proc_macro;

mod route;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Creates route handler with `GET` method guard.
///
/// Syntax: `#[get("path"[, attributes])]`
///
/// ## Attributes:
///
/// - `"path"` - Raw literal string with path for which to register handler. Mandatory.
/// - `guard = "function_name"` - Registers function as guard using `ntex::web::guard::fn_guard`
/// - `error = "ErrorRenderer"` - Register handler for different error renderer
#[proc_macro_attribute]
pub fn web_get(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Get) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `POST` method guard.
///
/// Syntax: `#[post("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn web_post(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Post) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `PUT` method guard.
///
/// Syntax: `#[put("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn web_put(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Put) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `DELETE` method guard.
///
/// Syntax: `#[delete("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn web_delete(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Delete) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `HEAD` method guard.
///
/// Syntax: `#[head("path"[, attributes])]`
///
/// Attributes are the same as in [head](attr.head.html)
#[proc_macro_attribute]
pub fn web_head(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Head) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `CONNECT` method guard.
///
/// Syntax: `#[connect("path"[, attributes])]`
///
/// Attributes are the same as in [connect](attr.connect.html)
#[proc_macro_attribute]
pub fn web_connect(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Connect) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `OPTIONS` method guard.
///
/// Syntax: `#[options("path"[, attributes])]`
///
/// Attributes are the same as in [options](attr.options.html)
#[proc_macro_attribute]
pub fn web_options(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Options) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `TRACE` method guard.
///
/// Syntax: `#[trace("path"[, attributes])]`
///
/// Attributes are the same as in [trace](attr.trace.html)
#[proc_macro_attribute]
pub fn web_trace(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Trace) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}

/// Creates route handler with `PATCH` method guard.
///
/// Syntax: `#[patch("path"[, attributes])]`
///
/// Attributes are the same as in [patch](attr.patch.html)
#[proc_macro_attribute]
pub fn web_patch(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = match route::Route::new(args, input, route::MethodType::Patch) {
        Ok(gen) => gen,
        Err(err) => return err.to_compile_error().into(),
    };
    gen.generate()
}
