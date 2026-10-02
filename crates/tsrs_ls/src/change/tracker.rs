// PARTIAL (see mod.rs): tracker.go, reduced to the Tracker fields NewTracker initializes and completions read.

use std::sync::Arc;

use tsrs_ast::NodeFactory;
use tsrs_core::context::Context;
use tsrs_core::{CompilerOptions, P};
use tsrs_printer::{self as printer, EmitContext};

use crate::completions::with_format_code_settings;
use crate::lsconv::Converters;
use crate::lsutil::FormatCodeSettings;

// tracker.go:83 (partial: the change lists, deleted nodes and insertion state are not ported yet)
pub struct Tracker {
    // initialized with
    pub(crate) format_settings: FormatCodeSettings,
    pub(crate) new_line: String,
    pub(crate) converters: Arc<Converters>,
    pub(crate) ctx: Context,
    pub emit_context: P<EmitContext>,

    pub node_factory: NodeFactory,
}

// tracker.go:112
pub fn new_tracker(ctx: &Context, compiler_options: &CompilerOptions, format_options: FormatCodeSettings, converters: Arc<Converters>) -> Tracker {
    let emit_context = printer::new_emit_context();
    let new_line = compiler_options.new_line.get_new_line_character().to_string();
    let ctx = with_format_code_settings(ctx, format_options.clone(), &new_line); // !!! formatSettings in context?
    Tracker {
        node_factory: emit_context.factory.as_node_factory().clone(),
        emit_context,
        ctx,
        converters,
        format_settings: format_options,
        new_line,
    }
}
