use tsrs_ast::{self as ast, Node, Symbol};
use tsrs_checker::Type;
use tsrs_core::context::Context;
use tsrs_core::P;
use tsrs_lsproto as lsproto;

use crate::astnav;
use crate::languageservice::LanguageService;

// api.go:13 (Go sentinel errors; `fmt.Errorf("%w: ...")` keeps the sentinel text as the message prefix)
pub const ERR_NO_SOURCE_FILE: &str = "source file not found";
pub const ERR_NO_TOKEN_AT_POSITION: &str = "no token found at position";

impl LanguageService {
    // api.go:18
    pub fn get_symbol_at_position(&self, ctx: &Context, file_name: &str, position: i32) -> Result<Option<P<Symbol>>, lsproto::Error> {
        let (program, file) = self.try_get_program_and_file(file_name);
        let Some(file) = file else {
            return Err(lsproto::Error::new(format!("{}: {}", ERR_NO_SOURCE_FILE, file_name)));
        };
        // Go checks the token for nil; astnav.GetTokenAtPosition never returns nil.
        let node = astnav::get_token_at_position(file, position);
        let mut checker = program.get_type_checker_for_file(ctx, file);
        Ok(checker.get_symbol_at_location_exported(node))
    }

    // api.go:32
    pub fn get_symbol_at_location(&self, ctx: &Context, node: P<Node>) -> Option<P<Symbol>> {
        let program = self.get_program();
        let mut checker = program.get_type_checker_for_file(ctx, ast::get_source_file_of_node(node).unwrap());
        checker.get_symbol_at_location_exported(node)
    }

    // api.go:39
    pub fn get_type_of_symbol(&self, ctx: &Context, symbol: P<Symbol>) -> Option<P<Type>> {
        let program = self.get_program();
        let mut checker = program.get_type_checker(ctx);
        checker.get_type_of_symbol_at_location(symbol, None)
    }
}
