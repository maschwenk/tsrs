mod jsdoc;
mod parser_1;
mod parser_2;
mod parser_3;
mod references;
mod reparser;
mod types;
mod utilities;

pub use parser_1::{parse_isolated_entity_name, parse_source_file, parse_source_file_owned, JSDocInfo, Parser, ParserState, ParsingContext, ParsingContexts};
pub use types::ParseFlags;
pub use utilities::get_jsdoc_comment_ranges;
