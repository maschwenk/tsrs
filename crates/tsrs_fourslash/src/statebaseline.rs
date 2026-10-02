// statebaseline.go: the project-state baseline of `// @stateBaseline: true` tests. Creating one needs
// fsbaselineutil.FSDiffer and the project / open-file / config-registry diff printers, which are not ported:
// NewFourslash fails such tests ("feature not ported: state baselines"), so the entry points below only run
// their "not enabled" path.

use tsrs_core::json::Value;
use tsrs_ls::lsconv;
use tsrs_lsproto as lsproto;

use crate::fourslash::FourslashTest;
use crate::testing::T;

// statebaseline.go:25
#[derive(Clone, Debug, Default)]
pub struct StateBaseline {
    pub(crate) baseline: String,
    pub(crate) is_initialized: bool,
    // !!! fsDiffer, serializedProjects, serializedOpenFiles, serializedConfigFileRegistry
}

impl FourslashTest {
    // statebaseline.go:51 (`params` is evaluated only when state baselining is enabled)
    pub(crate) fn baseline_request_or_notification(&mut self, t: &T, method: lsproto::Method, params: impl FnOnce() -> Value) {
        t.helper();

        if !self.test_data.is_state_baselining_enabled() {
            return;
        }

        // requestOrMessage{Method, Params} with `params,omitzero`
        let mut o = tsrs_core::collections::OrderedMap::default();
        o.insert("method".to_string(), Value::String(method.0.to_string()));
        let params = params();
        if params != Value::Null {
            o.insert("params".to_string(), params);
        }
        let res = tsrs_core::json::marshal_indent(&Value::Object(o), "", "  ").unwrap_or_default();
        let state_baseline = self.state_baseline.as_mut().unwrap();
        state_baseline.baseline.push('\n');
        state_baseline.baseline.push_str(&res);
        state_baseline.baseline.push('\n');
        state_baseline.is_initialized = true;
    }

    // statebaseline.go:66
    pub(crate) fn baseline_projects_after_notification(&mut self, t: &T, file_name: &str) {
        t.helper();
        if !self.test_data.is_state_baselining_enabled() {
            return;
        }
        // Do hover so we have snapshot to check things on!!
        let (_, result) = self.client().send_request(
            lsproto::TEXT_DOCUMENT_HOVER_INFO,
            lsproto::HoverParams {
                text_document: lsproto::TextDocumentIdentifier { uri: lsconv::file_name_to_document_uri(file_name) },
                position: lsproto::Position { line: 0, character: 0 },
                ..Default::default()
            },
        );
        crate::go::assert::assert(t, result.is_some(), "");
        self.baseline_state(t);
    }

    // statebaseline.go:85
    pub(crate) fn baseline_state(&mut self, t: &T) {
        t.helper();

        if !self.test_data.is_state_baselining_enabled() {
            return;
        }

        // !!! serializedState: fsDiffer.BaselineFSwithDiff + printStateDiff
        t.fatal("feature not ported: state baselines (statebaseline.go, fsbaselineutil)");
    }
}
