/// Used in the grammar for easy span creation
macro_rules! span {
    ($l:expr, $r:expr) => {
        ByteSpan::new($l, $r)
    };
    ($i:expr) => {
        ByteSpan::new($i, $i)
    };
}

/// Convenience function for building parser errors
macro_rules! to_lalrpop_err (
    ($error:expr) => (lalrpop_util::ParseError::User { error: $error })
);

#[cfg_attr(rustfmt, rustfmt_skip)]
#[allow(unknown_lints)]
#[allow(clippy)]
pub(crate) mod grammar {
    // During the build step, `build.rs` will output the generated parser to `OUT_DIR` to avoid
    // adding it to the source directory, so we just directly include the generated parser here.
    //
    // Even with `.gitignore` and the `exclude` in the `Cargo.toml`, the generated parser can still
    // end up in the source directory. This could happen when `cargo build` builds the file out of
    // the Cargo cache (`$HOME/.cargo/registrysrc`), and the build script would then put its output
    // in that cached source directory because of https://github.com/lalrpop/lalrpop/issues/280.
    // Later runs of `cargo vendor` then copy the source from that directory, including the
    // generated file.
    include!(concat!(env!("OUT_DIR"), "/parser/grammar.rs"));
}

#[macro_use]
mod macros;

pub mod ast;
mod errors;
/// Contains the visitor trait needed to traverse the AST and helper walk functions.
pub mod visitor;

use std::borrow::Cow;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use libeir_diagnostics::{CodeMap, FileName};

use crate::lexer::{FileMapSource, Lexer, Scanner, Source};
use crate::preprocessor::{MacroContainer, Preprocessed, Preprocessor};

pub use self::ast::{NodeId, NodeIdGenerator};
pub use self::errors::*;

/// The type of result returned from parsing functions
pub type ParseResult<T> = Result<T, Vec<ParserError>>;

pub struct Parser {
    pub config: ParseConfig,
}
impl Parser {
    pub fn new(config: ParseConfig) -> Parser {
        Parser { config }
    }

    pub fn parse_string<S, T>(&self, source: S) -> ParseResult<T>
    where
        S: AsRef<str>,
        T: Parse,
    {
        let filemap = {
            self.config.codemap.lock().unwrap().add_filemap(
                FileName::Virtual(Cow::Borrowed("nofile")),
                source.as_ref().to_owned(),
            )
        };
        <T as Parse<T>>::parse(&self.config, FileMapSource::new(filemap))
    }

    pub fn parse_file<P, T>(&self, path: P) -> ParseResult<T>
    where
        P: AsRef<Path>,
        T: Parse,
    {
        match FileMapSource::from_path(self.config.codemap.clone(), path) {
            Err(err) => return Err(vec![err.into()]),
            Ok(source) => <T as Parse<T>>::parse(&self.config, source),
        }
    }
}

pub struct ParseConfig {
    pub codemap: Arc<Mutex<CodeMap>>,
    pub warnings_as_errors: bool,
    pub no_warn: bool,
    pub include_paths: VecDeque<PathBuf>,
    pub code_paths: VecDeque<PathBuf>,
    pub macros: Option<MacroContainer>,
}
impl ParseConfig {
    pub fn new(codemap: Arc<Mutex<CodeMap>>) -> Self {
        ParseConfig {
            codemap,
            warnings_as_errors: false,
            no_warn: false,
            include_paths: VecDeque::new(),
            code_paths: VecDeque::new(),
            macros: None,
        }
    }
}
impl Default for ParseConfig {
    fn default() -> Self {
        ParseConfig {
            codemap: Arc::new(Mutex::new(CodeMap::new())),
            warnings_as_errors: false,
            no_warn: false,
            include_paths: VecDeque::new(),
            code_paths: VecDeque::new(),
            macros: None,
        }
    }
}

pub trait Parse<T = Self> {
    type Parser;

    /// Initializes a token stream for the underlying parser and invokes parse_tokens/1
    fn parse<S>(config: &ParseConfig, source: S) -> ParseResult<T>
    where
        S: Source,
    {
        let scanner = Scanner::new(source);
        let lexer = Lexer::new(scanner);
        let tokens = Preprocessor::new(config, lexer);
        let mut nid = NodeIdGenerator::new();
        Self::parse_tokens(&mut nid, tokens)
    }

    /// Implemented by each parser, which should parse the token stream and produce a T
    fn parse_tokens<S: IntoIterator<Item = Preprocessed>>(nid: &mut NodeIdGenerator, tokens: S) -> ParseResult<T>;
}

impl Parse for ast::Module {
    type Parser = grammar::ModuleParser;

    fn parse_tokens<S: IntoIterator<Item = Preprocessed>>(nid: &mut NodeIdGenerator, tokens: S) -> ParseResult<ast::Module> {
        let mut errs = Vec::new();
        let result = Self::Parser::new()
            .parse(&mut errs, nid, tokens)
            .map_err(|e| e.map_error(|ei| ei.into()));
        to_parse_result(errs, result)
    }
}

impl Parse for ast::Expr {
    type Parser = grammar::ExprParser;

    fn parse_tokens<S: IntoIterator<Item = Preprocessed>>(nid: &mut NodeIdGenerator, tokens: S) -> ParseResult<ast::Expr> {
        let mut errs = Vec::new();
        let result = Self::Parser::new()
            .parse(&mut errs, nid, tokens)
            .map_err(|e| e.map_error(|ei| ei.into()));
        to_parse_result(errs, result)
    }
}

fn to_parse_result<T>(mut errs: Vec<ParseError>, result: Result<T, ParseError>) -> ParseResult<T> {
    match result {
        Ok(ast) => {
            if errs.len() > 0 {
                return Err(errs.drain(0..).map(ParserError::from).collect());
            }
            Ok(ast)
        }
        Err(err) => {
            errs.push(err);
            Err(errs.drain(0..).map(ParserError::from).collect())
        }
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::ast::*;
    use super::*;

    use libeir_diagnostics::ByteSpan;
    use libeir_diagnostics::{ColorChoice, Emitter, StandardStreamEmitter};

    use crate::lexer::{Ident, Symbol};
    use crate::preprocessor::PreprocessorError;

    fn parse<T>(input: &'static str) -> T
    where
        T: Parse<T>,
    {
        let config = ParseConfig::default();
        let parser = Parser::new(config);
        let errs = match parser.parse_string::<&'static str, T>(input) {
            Ok(ast) => return ast,
            Err(errs) => errs,
        };
        let emitter = StandardStreamEmitter::new(ColorChoice::Auto)
            .set_codemap(parser.config.codemap.clone());
        for err in errs.iter() {
            emitter.diagnostic(&err.to_diagnostic()).unwrap();
        }
        panic!("parse failed");
    }

    fn parse_fail<T>(input: &'static str) -> Vec<ParserError>
    where
        T: Parse<T>,
    {
        let config = ParseConfig::default();
        let parser = Parser::new(config);
        match parser.parse_string::<&'static str, T>(input) {
            Err(errs) => errs,
            _ => panic!("expected parse to fail, but it succeeded!"),
        }
    }

    macro_rules! module {
        ($nid:expr, $name:expr, $body:expr) => {{
            let mut errs = Vec::new();
            let module = Module::new(&mut errs, ByteSpan::default(), $nid, $name, $body);
            if errs.len() > 0 {
                let emitter = StandardStreamEmitter::new(ColorChoice::Auto);
                for err in errs.drain(..) {
                    let err = ParserError::from(err);
                    emitter.diagnostic(&err.to_diagnostic()).unwrap();
                }
                panic!("failed to create expected module!");
            }
            module
        }};
    }

    #[test]
    fn parse_empty_module() {
        let result: Module = parse("-module(foo).");
        let mut nid = NodeIdGenerator::new();
        let expected = module!(&mut nid, ident!("foo"), vec![]);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_module_with_multi_clause_function() {
        let result: Module = parse(
            "-module(foo).

foo([], Acc) -> Acc;
foo([H|T], Acc) -> foo(T, [H|Acc]).
",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(foo),
            params: vec![nil!(nid), var!(nid, Acc)],
            guard: None,
            body: vec![var!(nid, Acc)],
        });
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(foo),
            params: vec![cons!(nid, var!(nid, H), var!(nid, T)), var!(nid, Acc)],
            guard: None,
            body: vec![apply!(nid, atom!(nid, foo), var!(nid, T), cons!(nid, var!(nid, H), var!(nid, Acc)))],
        });
        let mut body = Vec::new();
        body.push(TopLevel::Function(NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!("foo"),
            arity: 2,
            clauses,
            spec: None,
        }));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_if_expressions() {
        let result: Module = parse(
            "-module(foo).

unless(false) ->
    true;
unless(true) ->
    false;
unless(Value) ->
    if
        Value == 0 -> true;
        Value -> false;
        else -> true
    end.

",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(unless),
            params: vec![atom!(nid, false)],
            guard: None,
            body: vec![atom!(nid, true)],
        });
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(unless),
            params: vec![atom!(nid, true)],
            guard: None,
            body: vec![atom!(nid, false)],
        });
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(unless),
            params: vec![var!(nid, Value)],
            guard: None,
            body: vec![Expr::If(If {
                span: ByteSpan::default(),
                id: nid.next(),
                clauses: vec![
                    IfClause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        guards: vec![
                            Guard {
                                span: ByteSpan::default(),
                                conditions: vec![Expr::BinaryExpr(BinaryExpr {
                                    span: ByteSpan::default(),
                                    id: nid.next(),
                                    lhs: Box::new(var!(nid, Value)),
                                    op: BinaryOp::Equal,
                                    rhs: Box::new(int!(nid, 0)),
                                })],
                            },
                        ],
                        body: vec![atom!(nid, true)],
                    },
                    IfClause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        guards: vec![
                            Guard {
                                span: ByteSpan::default(),
                                conditions: vec![var!(nid, Value)],
                            },
                        ],
                        body: vec![atom!(nid, false)],
                    },
                    IfClause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        guards: vec![
                            Guard {
                                span: ByteSpan::default(),
                                conditions: vec![atom!(nid, else)],
                            },
                        ],
                        body: vec![atom!(nid, true)],
                    },
                ],
            })],
        });
        let mut body = Vec::new();
        body.push(TopLevel::Function(NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(unless),
            arity: 1,
            clauses,
            spec: None,
        }));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_case_expressions() {
        let result: Module = parse(
            "-module(foo).

typeof(Value) ->
    case Value of
        [] -> nil;
        [_|_] -> list;
        N when is_number(N) -> N;
        _ -> other
    end.

",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(typeof),
            params: vec![var!(nid, Value)],
            guard: None,
            body: vec![Expr::Case(Case {
                span: ByteSpan::default(),
                id: nid.next(),
                expr: Box::new(var!(nid, Value)),
                clauses: vec![
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: nil!(nid),
                        guard: None,
                        body: vec![atom!(nid, nil)],
                    },
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: cons!(nid, var!(nid, _), var!(nid, _)),
                        guard: None,
                        body: vec![atom!(nid, list)],
                    },
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: var!(nid, N),
                        guard: Some(vec![Guard {
                            span: ByteSpan::default(),
                            conditions: vec![apply!(nid, atom!(nid, is_number), var!(nid, N))],
                        }]),
                        body: vec![var!(nid, N)],
                    },
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: var!(nid, _),
                        guard: None,
                        body: vec![atom!(nid, other)],
                    },
                ],
            })],
        });
        let mut body = Vec::new();
        body.push(TopLevel::Function(NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(typeof),
            arity: 1,
            clauses,
            spec: None,
        }));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_receive_expressions() {
        let result: Module = parse(
            "-module(foo).

loop(State, Timeout) ->
    receive
        {From, {Ref, Msg}} ->
            From ! {Ref, ok},
            handle_info(Msg, State);
        _ ->
            exit(io_lib:format(\"unexpected message: ~p~n\", [Msg]))
    after
        Timeout ->
            timeout
    end.
",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(loop),
            params: vec![var!(nid, State), var!(nid, Timeout)],
            guard: None,
            body: vec![Expr::Receive(Receive {
                span: ByteSpan::default(),
                id: nid.next(),
                clauses: Some(vec![
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: tuple!(nid, var!(nid, From), tuple!(nid, var!(nid, Ref), var!(nid, Msg))),
                        guard: None,
                        body: vec![
                            Expr::BinaryExpr(BinaryExpr {
                                span: ByteSpan::default(),
                                id: nid.next(),
                                lhs: Box::new(var!(nid, From)),
                                op: BinaryOp::Send,
                                rhs: Box::new(tuple!(nid, var!(nid, Ref), atom!(nid, ok))),
                            }),
                            apply!(nid, atom!(nid, handle_info), var!(nid, Msg), var!(nid, State)),
                        ],
                    },
                    Clause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        pattern: var!(nid, _),
                        guard: None,
                        body: vec![apply!(
                            nid,
                            atom!(nid, exit),
                            apply!(
                                nid,
                                remote!(nid, io_lib, format),
                                Expr::Literal(Literal::String(nid.next(), ident!("unexpected message: ~p~n"))),
                                cons!(nid, var!(nid, Msg), nil!(nid))
                            )
                        )],
                    },
                ]),
                after: Some(After {
                    span: ByteSpan::default(),
                    id: nid.next(),
                    timeout: Box::new(var!(nid, Timeout)),
                    body: vec![atom!(nid, timeout)],
                }),
            })],
        });
        let mut body = Vec::new();
        body.push(TopLevel::Function(NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(loop),
            arity: 2,
            clauses,
            spec: None,
        }));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_preprocessor_if() {
        let result: Module = parse(
            "-module(foo).
-define(TEST, true).
-define(OTP_VERSION, 21).

-ifdef(TEST).
env() ->
    test.
-else.
env() ->
    release.
-endif.

-if(?OTP_VERSION > 21).
system_version() ->
    future.
-elif(?OTP_VERSION == 21).
system_version() ->
    ?OTP_VERSION.
-else.
system_version() ->
    old.
-endif.
",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut body = Vec::new();
        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(env),
            params: vec![],
            guard: None,
            body: vec![atom!(nid, test)],
        });
        let env_fun = NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(env),
            arity: 0,
            clauses,
            spec: None,
        };
        body.push(TopLevel::Function(env_fun));

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(system_version),
            params: vec![],
            guard: None,
            body: vec![int!(nid, 21)],
        });
        let system_version_fun = NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(system_version),
            arity: 0,
            clauses,
            spec: None,
        };
        body.push(TopLevel::Function(system_version_fun));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_preprocessor_warning_error() {
        // NOTE: Warnings are not printed with cfg(test), as we
        // cannot control where they end up without refactoring to pass
        // a writer everywhere. You can change this for testing by
        // going to the Preprocessor and finding the line where we handle
        // the warning directive and toggle the config flag
        let mut errs = parse_fail::<Module>(
            "-module(foo).
-warning(\"this is a compiler warning\").
-error(\"this is a compiler error\").
",
        );
        match errs.pop() {
            Some(ParserError::Preprocessor(PreprocessorError::CompilerError(_, _))) => (),
            Some(err) => panic!(
                "expected compiler error, but got a different error instead: {:?}",
                err
            ),
            None => panic!("expected compiler error, but didn't get any errors!"),
        }
    }

    #[test]
    fn parse_try() {
        let result: Module = parse(
            "-module(foo).

example(File) ->
    try read(File) of
        {ok, Contents} ->
            {ok, Contents}
    catch
        error:{Mod, Code} ->
            {error, Mod:format_error(Code)};
        Reason ->
            {error, Reason}
    after
        close(File)
    end.
",
        );

        let mut id_gen = NodeIdGenerator::new();
        let nid = &mut id_gen;

        let mut clauses = Vec::new();
        clauses.push(FunctionClause {
            span: ByteSpan::default(),
            name: ident_opt!(example),
            params: vec![var!(nid, File)],
            guard: None,
            body: vec![Expr::Try(Try {
                span: ByteSpan::default(),
                id: nid.next(),
                exprs: vec![apply!(nid, atom!(nid, read), var!(nid, File))],
                clauses: Some(vec![Clause {
                    span: ByteSpan::default(),
                    id: nid.next(),
                    pattern: tuple!(nid, atom!(nid, ok), var!(nid, Contents)),
                    guard: None,
                    body: vec![tuple!(nid, atom!(nid, ok), var!(nid, Contents))],
                }]),
                catch_clauses: Some(vec![
                    TryClause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        kind: Name::Atom(ident!(error)),
                        error: tuple!(nid, var!(nid, Mod), var!(nid, Code)),
                        trace: ident!(_),
                        guard: None,
                        body: vec![tuple!(
                            nid,
                            atom!(nid, error),
                            apply!(nid, remote!(nid, var!(nid, Mod), atom!(nid, format_error)), var!(nid, Code))
                        )],
                    },
                    TryClause {
                        span: ByteSpan::default(),
                        id: nid.next(),
                        kind: Name::Atom(ident!(throw)),
                        error: var!(nid, Reason),
                        trace: ident!(_),
                        guard: None,
                        body: vec![tuple!(nid, atom!(nid, error), var!(nid, Reason))],
                    },
                ]),
                after: Some(vec![apply!(nid, atom!(nid, close), var!(nid, File))]),
            })],
        });
        let mut body = Vec::new();
        body.push(TopLevel::Function(NamedFunction {
            span: ByteSpan::default(),
            id: nid.next(),
            name: ident!(example),
            arity: 1,
            clauses,
            spec: None,
        }));
        let expected = module!(nid, ident!(foo), body);
        assert_eq!(result, expected);
    }

    #[test]
    fn parse_try2() {
        let _result: Module = parse(
            "-module(foo).

example(File < 2) ->
    try read(File) of
        {ok, Contents} ->
            {ok, Contents}
    catch
        error:{Mod, Code} ->
            {error, Mod:format_error(Code)};
        Reason ->
            {error, Reason}
    after
        close(File)
    end.

exw(File) ->
    case File of
        File < 2 ->
            ok
    end.
",
        );
    }

    #[test]
    fn parse_numbers() {
        let _result: Module = parse(
            "-module(foo).

foo(F) -> F-1+1/1*1.

bar() -> - 2.
",
        );
    }

    #[test]
    fn parse_spec() {
        let _result: Module = parse(
            "-module(foo).

-spec bar() -> number.
bar() -> 2.
",
        );
    }

}