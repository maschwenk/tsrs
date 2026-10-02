use tsrs_ast::{Kind, Symbol, SymbolFlags};
use tsrs_core::{stringutil, UTF16Offset, P};
use tsrs_lsproto as lsproto;
use tsrs_printer::EmitTextWriter;

// displayPartsWriter implements EmitTextWriter and captures classified text runs
// for VS colorized labels, while also building a plain string.
// When vsCapability is false, only the plain string is built; runs are skipped.
// displaypartswriter.go:19
pub(crate) struct DisplayPartsWriter {
    builder: String,
    runs: Vec<lsproto::VSClassifiedTextRun>,
    vs_capability: bool,
    last_written: String,
}

// displaypartswriter.go:26
pub(crate) fn new_display_parts_writer(vs_capability: bool) -> DisplayPartsWriter {
    DisplayPartsWriter { builder: String::new(), runs: Vec::new(), vs_capability, last_written: String::new() }
}

impl DisplayPartsWriter {
    // displaypartswriter.go:30
    fn add_run(&mut self, classification: lsproto::ClassificationTypeName, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.vs_capability {
            self.runs.push(lsproto::VSClassifiedTextRun {
                classification_type_name: classification.0.to_string(),
                text: text.to_string(),
                ..Default::default()
            });
        }
        self.last_written = text.to_string();
        self.builder.push_str(text);
    }

    // WriteClassified writes text with an explicit classification type.
    // displaypartswriter.go:45
    pub(crate) fn write_classified(&mut self, text: &str, classification: lsproto::ClassificationTypeName) {
        self.add_run(classification, text);
    }

    // WriteFrom copies the accumulated content from another displayPartsWriter.
    // displaypartswriter.go:50
    pub(crate) fn write_from(&mut self, other: &DisplayPartsWriter) {
        self.builder.push_str(&other.string());
        if self.vs_capability {
            self.runs.extend_from_slice(other.get_runs());
        }
        if !other.last_written.is_empty() {
            self.last_written = other.last_written.clone();
        }
    }

    // displaypartswriter.go:60
    pub(crate) fn get_runs(&self) -> &[lsproto::VSClassifiedTextRun] {
        &self.runs
    }

    // displaypartswriter.go:64 (Go String())
    pub(crate) fn as_str(&self) -> &str {
        &self.builder
    }
}

impl EmitTextWriter for DisplayPartsWriter {
    // displaypartswriter.go:64
    fn string(&self) -> String {
        self.builder.clone()
    }

    // displaypartswriter.go:68
    fn clear(&mut self) {
        self.last_written.clear();
        self.builder.clear();
        self.runs = Vec::new();
    }

    // displaypartswriter.go:74
    fn decrease_indent(&mut self) {}

    // displaypartswriter.go:76
    fn get_column(&self) -> UTF16Offset {
        0
    }

    // displaypartswriter.go:78
    fn get_indent(&self) -> i32 {
        0
    }

    // displaypartswriter.go:80
    fn get_line(&self) -> i32 {
        0
    }

    // displaypartswriter.go:82
    fn get_text_pos(&self) -> i32 {
        self.builder.len() as i32
    }

    // displaypartswriter.go:86
    fn has_trailing_comment(&self) -> bool {
        false
    }

    // displaypartswriter.go:88
    fn has_trailing_whitespace(&self) -> bool {
        if self.builder.is_empty() {
            return false;
        }
        let (ch, _) = stringutil::decode_last_rune(self.last_written.as_bytes());
        if ch == 0xFFFD /* utf8.RuneError */ {
            return false;
        }
        stringutil::is_white_space_like(ch)
    }

    // displaypartswriter.go:99
    fn increase_indent(&mut self) {}

    // displaypartswriter.go:101
    fn is_at_start_of_line(&self) -> bool {
        false
    }

    // displaypartswriter.go:103
    fn raw_write(&mut self, s: &str) {
        self.add_run(lsproto::ClassificationTypeName::Text, s);
    }

    // displaypartswriter.go:107
    fn write(&mut self, s: &str) {
        self.add_run(lsproto::ClassificationTypeName::Text, s);
    }

    // displaypartswriter.go:111
    fn write_comment(&mut self, text: &str) {
        // Strada's writeComment uses unknownWrite → SymbolDisplayPartKind.text → "text"
        self.add_run(lsproto::ClassificationTypeName::Text, text);
    }

    // displaypartswriter.go:116
    fn write_keyword(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::Keyword, text);
    }

    // displaypartswriter.go:120
    fn write_line(&mut self) {
        self.add_run(lsproto::ClassificationTypeName::WhiteSpace, " ");
    }

    // displaypartswriter.go:124
    fn write_line_force(&mut self, _force: bool) {
        self.add_run(lsproto::ClassificationTypeName::WhiteSpace, " ");
    }

    // displaypartswriter.go:128
    fn write_literal(&mut self, s: &str) {
        // Strada's writeLiteral → SymbolDisplayPartKind.stringLiteral → "string"
        self.add_run(lsproto::ClassificationTypeName::String, s);
    }

    // displaypartswriter.go:133
    fn write_operator(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::Operator, text);
    }

    // displaypartswriter.go:137
    fn write_parameter(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::ParameterName, text);
    }

    // displaypartswriter.go:141
    fn write_property(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::PropertyName, text);
    }

    // displaypartswriter.go:145
    fn write_punctuation(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::Punctuation, text);
    }

    // displaypartswriter.go:149
    fn write_space(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::WhiteSpace, text);
    }

    // displaypartswriter.go:153
    fn write_string_literal(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::String, text);
    }

    // displaypartswriter.go:157
    fn write_symbol(&mut self, text: &str, symbol: P<Symbol>) {
        let classification = classification_for_symbol(Some(symbol));
        self.add_run(classification, text);
    }

    // displaypartswriter.go:162
    fn write_trailing_semicolon(&mut self, text: &str) {
        self.add_run(lsproto::ClassificationTypeName::Punctuation, text);
    }
}

// classificationForSymbol determines the Roslyn classification type name based on a symbol's flags.
// Matches the Strada translation chain: displayPartKind() → GetClassificationName().
// displaypartswriter.go:168
fn classification_for_symbol(symbol: Option<P<Symbol>>) -> lsproto::ClassificationTypeName {
    let Some(symbol) = symbol else {
        return lsproto::ClassificationTypeName::Text;
    };
    let flags = symbol.flags();
    if flags.intersects(SymbolFlags::Variable) {
        if is_first_declaration_of_symbol_parameter(symbol) {
            return lsproto::ClassificationTypeName::ParameterName;
        }
        lsproto::ClassificationTypeName::LocalName
    } else if flags.intersects(SymbolFlags::Property) {
        lsproto::ClassificationTypeName::PropertyName
    } else if flags.intersects(SymbolFlags::GetAccessor) {
        lsproto::ClassificationTypeName::PropertyName
    } else if flags.intersects(SymbolFlags::SetAccessor) {
        lsproto::ClassificationTypeName::PropertyName
    } else if flags.intersects(SymbolFlags::EnumMember) {
        lsproto::ClassificationTypeName::FieldName
    } else if flags.intersects(SymbolFlags::Function) {
        lsproto::ClassificationTypeName::MethodName
    } else if flags.intersects(SymbolFlags::Class) {
        lsproto::ClassificationTypeName::ClassName
    } else if flags.intersects(SymbolFlags::Interface) {
        lsproto::ClassificationTypeName::InterfaceName
    } else if flags.intersects(SymbolFlags::Enum) {
        lsproto::ClassificationTypeName::EnumName
    } else if flags.intersects(SymbolFlags::Module) {
        lsproto::ClassificationTypeName::ModuleName
    } else if flags.intersects(SymbolFlags::Method) {
        lsproto::ClassificationTypeName::MethodName
    } else if flags.intersects(SymbolFlags::TypeParameter) {
        lsproto::ClassificationTypeName::TypeParameterName
    } else if flags.intersects(SymbolFlags::TypeAlias) {
        lsproto::ClassificationTypeName::Identifier
    } else if flags.intersects(SymbolFlags::Alias) {
        lsproto::ClassificationTypeName::Identifier
    } else {
        lsproto::ClassificationTypeName::Text
    }
}

// isFirstDeclarationOfSymbolParameter checks if the symbol's first declaration is a parameter.
// displaypartswriter.go:211
fn is_first_declaration_of_symbol_parameter(symbol: P<Symbol>) -> bool {
    let declarations = symbol.declarations();
    if declarations.is_empty() {
        return false;
    }
    declarations[0].kind() == Kind::Parameter
}
