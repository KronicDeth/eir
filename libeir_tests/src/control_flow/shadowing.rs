use crate::lower;

use libeir_intern::Ident;
use libeir_ir::FunctionIdent;
use libeir_passes::PassManager;
use libeir_syntax_erl::ParseConfig;

use libeir_interpreter::VMState;

#[test]
fn pattern_variable_shadowing() {
    let _ = env_logger::try_init();

    let mut eir_mod = lower(
        "-module(shadowing).

run() ->
    1 = case_matching({1}, 1),
    {_, _} = (catch case_matching({1}, 2)),

    1 = fun_shadowing({1}, 1),
    2 = fun_shadowing({2}, 1).

case_matching(A, B) ->
    case A of
        {B} -> B
    end.

fun_shadowing(A, B) ->
    C = fun({B}) -> B end,
    C(A).

",
        ParseConfig::default(),
    )
    .unwrap();

    let mut pass_manager = PassManager::default();
    pass_manager.run(&mut eir_mod);

    let mut vm = VMState::new();
    vm.add_builtin_modules();
    vm.add_erlang_module(eir_mod);

    let run_fun = FunctionIdent {
        module: Ident::from_str("shadowing"),
        name: Ident::from_str("run"),
        arity: 0,
    };
    assert!(vm.call(&run_fun, &[]).is_ok());
}

#[test]
fn pattern_variable_shadowing_a() {
    let _ = env_logger::try_init();

    let mut eir_mod = lower(
        "-module(shadowinga).

fun_shadowing(A) ->
    C = fun(B) -> B end,
    C(A).

",
        ParseConfig::default(),
    )
    .unwrap();

    let mut pass_manager = PassManager::default();
    pass_manager.run(&mut eir_mod);

    for fun_def in eir_mod.function_iter() {
        let fun = fun_def.function();
        fun.live_values();
    }
}
