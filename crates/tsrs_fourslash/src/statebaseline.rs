// statebaseline.go: the project-state baseline of `// @statebaseline: true` tests. Everything it records comes
// from the server's session (projects, open files, config file registry, file system writes), so only the
// accumulated text is kept for now.

// statebaseline.go:25
#[derive(Clone, Debug, Default)]
pub struct StateBaseline {
    pub(crate) baseline: String,
    pub(crate) is_initialized: bool,
    // !!! fsDiffer, serializedProjects, serializedOpenFiles, serializedConfigFileRegistry (need the server)
}
