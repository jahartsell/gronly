//! Procedural macros for `gronly-atomics`

use darling::ast::NestedMeta;
use darling::FromMeta;
use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::parse_macro_input;

/// Generates a test that is only run un-modeled
///
/// # Options
///
///  * `notest`
///
///    Do not add the `#[test]` attribute, useful if using a custom testing macro e.g. `#[rstest]`
///
///  * `shuttle(scheduler = "name", iters = N, depth = N)`
///
///    Configure the scheduler for shuttle. Defaults to "random" with 100 iterations. Valid options
///    for scheduler are "random", "pct", "dfs", and "relpay"
#[proc_macro_attribute]
pub fn unmodeled_test(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let metalist = match NestedMeta::parse_meta_list(args.into()) {
        Ok(v) => v,
        Err(e) => {
            return e.into_compile_error().into();
        }
    };

    let args = match UnmodeledTestArgs::from_list(&metalist) {
        Ok(v) => v,
        Err(e) => {
            return e.write_errors().into();
        }
    };

    let testfn = parse_macro_input!(input as TestFn);
    render_native(args.notest, &testfn).into()
}

#[derive(PartialEq, Eq, Debug, Default, FromMeta)]
struct UnmodeledTestArgs {
    #[darling(default)]
    notest: bool,
}

/// Generates modeled tests using `loom` and `shuttle`
///
/// # Options
///
///  * `notest`
///
///    Do not add the `#[test]` attribute, useful if using a custom testing macro e.g. `#[rstest]`
///
///  * `shuttle(scheduler = "name", iters = N, depth = N)`
///
///    Configure the scheduler for shuttle. Defaults to "random" with 100 iterations. Valid options
///    for scheduler are "random", "pct", "dfs", and "relpay"
#[proc_macro_attribute]
pub fn modeled_test(
    args: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let metalist = match NestedMeta::parse_meta_list(args.into()) {
        Ok(v) => v,
        Err(e) => {
            return e.into_compile_error().into();
        }
    };

    let args = match ModeledTestArgs::from_list(&metalist) {
        Ok(v) => v,
        Err(e) => {
            return e.write_errors().into();
        }
    };

    let testfn = parse_macro_input!(input as TestFn);
    render_modeled_test(&args, &testfn).into()
}

#[derive(PartialEq, Eq, Debug, Default, FromMeta)]
struct ModeledTestArgs {
    #[darling(default)]
    notest: bool,

    #[darling(default)]
    shuttle: ShuttleArgs,
}

#[derive(PartialEq, Eq, Debug, Default, FromMeta)]
struct ShuttleArgs {
    #[darling(default)]
    scheduler: Scheduler,

    #[darling(default)]
    iters: Option<usize>,

    #[darling(default)]
    depth: Option<usize>,
}

#[derive(PartialEq, Eq, Debug, Default, FromMeta)]
enum Scheduler {
    #[default]
    Random,
    Pct,
    Dfs,
    Replay(String),
}

fn render_modeled_test(args: &ModeledTestArgs, testfn: &TestFn) -> TokenStream {
    [
        render_native(args.notest, &testfn),
        render_loom(&args, &testfn),
        render_shuttle(&args, &testfn),
    ]
    .into_iter()
    .collect()
}

fn render_header(notest: bool, testfn: &TestFn, cfg_guard: TokenStream) -> TokenStream {
    let test_attr = (!notest).then(|| quote! { #[test] });
    let attrs = &testfn.attrs;
    let vis = &testfn.vis;
    let sig = &testfn.sig;
    quote! {
        #cfg_guard #test_attr #(#attrs)* #vis # sig
    }
}

fn render_native(notest: bool, testfn: &TestFn) -> TokenStream {
    let cfg = quote! { #[cfg(not(any(loom, shuttle)))] };
    let header = render_header(notest, &testfn, cfg);
    let block = &testfn.block;
    quote! {
        #header #block
    }
}

fn render_loom(args: &ModeledTestArgs, testfn: &TestFn) -> TokenStream {
    let cfg = quote! { #[cfg(loom)] };
    let header = render_header(args.notest, &testfn, cfg);
    let block = &testfn.block;
    quote! {
        #header {
            ::gronly_atomics::model(|| #block )
        }
    }
}

fn render_shuttle(args: &ModeledTestArgs, testfn: &TestFn) -> TokenStream {
    let cfg = quote! { #[cfg(shuttle)] };
    let header = render_header(args.notest, &testfn, cfg);
    let block = &testfn.block;
    let model = match &args.shuttle.scheduler {
        Scheduler::Random => {
            let iters = args.shuttle.iters.unwrap_or(100);
            quote! { ::gronly_atomics::check_random(|| #block, #iters )}
        }
        Scheduler::Dfs => {
            let iters = args.shuttle.iters;
            quote! { ::gronly_atomics::check_dfs(|| #block, #iters )}
        }
        Scheduler::Pct => {
            let iters = args.shuttle.iters.unwrap_or(100);
            let depth = args.shuttle.depth.unwrap_or(100);
            quote! { ::gronly_atomics::check_pct(|| #block, #iters, #depth )}
        }
        Scheduler::Replay(name) => {
            quote! { ::gronly_atomics::replay(|| #block, #name )}
        }
    };
    quote! { #header { #model } }
}

/// Syntax node for a test function
///
/// Similar to `syn::ItemFn` but blindly forwards the function block to avoid excessive parsing
struct TestFn {
    attrs: Vec<syn::Attribute>,
    vis: syn::Visibility,
    sig: syn::Signature,
    block: TokenStream,
}

impl Parse for TestFn {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(syn::Attribute::parse_outer)?;
        let vis = input.parse()?;
        let sig = input.parse()?;
        let block = input.parse()?;
        Ok(Self {
            attrs,
            vis,
            sig,
            block,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use quote::quote;
    use syn::{parse::Parser, parse_quote};

    fn assert_tokenstream_eq(lhs: TokenStream, rhs: TokenStream) {
        let lhs = syn::File::parse.parse2(lhs).unwrap();
        let rhs = syn::File::parse.parse2(rhs).unwrap();
        assert_eq!(prettyplease::unparse(&lhs), prettyplease::unparse(&rhs));
    }

    #[test]
    fn args_defaults() {
        let args = ModeledTestArgs::from_list(&[]).unwrap();

        let expected = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Random,
                iters: None,
                depth: None,
            },
        };
        assert_eq!(args, expected);
    }

    #[test]
    fn args_parses_metalist() {
        let tokens = quote! { notest, shuttle(iters = 10, scheduler = "pct") };
        let meta = NestedMeta::parse_meta_list(tokens).unwrap();
        let args = ModeledTestArgs::from_list(&meta).unwrap();

        let expected = ModeledTestArgs {
            notest: true,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Pct,
                iters: Some(10),
                depth: None,
            },
        };
        assert_eq!(args, expected);
    }

    #[test]
    fn args_parses_replay() {
        let tokens = quote! { shuttle(scheduler(replay = "hello")) };
        let meta = NestedMeta::parse_meta_list(tokens).unwrap();
        let args = match ModeledTestArgs::from_list(&meta) {
            Ok(v) => v,
            Err(e) => {
                panic!("{}", e.write_errors().to_string());
            }
        };

        let expected = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Replay("hello".into()),
                iters: None,
                depth: None,
            },
        };
        assert_eq!(args, expected);
    }

    #[test]
    fn render_native_works() {
        let args = ModeledTestArgs::default();
        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(not(any(loom, shuttle)))] #[test] #[hello] #[world]
            fn do_thing() {
                assert!(true);
            }
        };

        let rendered = render_native(args.notest, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_native_omits_test() {
        let mut args = ModeledTestArgs::default();
        args.notest = true;

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(not(any(loom, shuttle)))] #[hello] #[world]
            fn do_thing() {
                assert!(true);
            }
        };

        let rendered = render_native(args.notest, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_loom_works() {
        let args = ModeledTestArgs::default();

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(loom)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::model(|| {
                    assert!(true);
                })
            }
        };

        let rendered = render_loom(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_shuttle_random() {
        let args = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Random,
                iters: Some(10),
                depth: None,
            },
        };

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(shuttle)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::check_random(|| {
                    assert!(true);
                }, 10usize)
            }
        };

        let rendered = render_shuttle(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_shuttle_pct() {
        let args = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Pct,
                iters: Some(10),
                depth: Some(10),
            },
        };

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(shuttle)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::check_pct(|| {
                    assert!(true);
                }, 10usize, 10usize)
            }
        };

        let rendered = render_shuttle(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_shuttle_dfs() {
        let args = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Dfs,
                iters: Some(10),
                depth: None,
            },
        };

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(shuttle)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::check_dfs(|| {
                    assert!(true);
                }, 10usize)
            }
        };

        let rendered = render_shuttle(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_shuttle_replay() {
        let args = ModeledTestArgs {
            notest: false,
            shuttle: ShuttleArgs {
                scheduler: Scheduler::Replay("myreplay".into()),
                iters: None,
                depth: None,
            },
        };

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(shuttle)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::replay(|| {
                    assert!(true);
                }, "myreplay")
            }
        };

        let rendered = render_shuttle(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }

    #[test]
    fn render_modeled_test_works() {
        let args = ModeledTestArgs::default();

        let input: TestFn = parse_quote! {
            #[hello] #[world] fn do_thing() {
                assert!(true);
            }
        };

        let expected = quote! {
            #[cfg(not(any(loom, shuttle)))] #[test] #[hello] #[world]
            fn do_thing() {
                assert!(true);
            }

            #[cfg(loom)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::model(|| {
                    assert!(true);
                })
            }

            #[cfg(shuttle)] #[test] #[hello] #[world]
            fn do_thing() {
                ::gronly_atomics::check_random(|| {
                    assert!(true);
                }, 100usize)
            }
        };

        let rendered = render_modeled_test(&args, &input);
        assert_tokenstream_eq(rendered, expected);
    }
}
