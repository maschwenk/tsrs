// buildinfo_contentmapper_test.go (TestStaticContentMapperTransformIdentity is in tsrs_tsoptions'
// contentmappers_test.rs): content mapper identities in the build info decide whether the old program is reused.

use std::sync::Arc;

use tsrs_contentmapper::{self as contentmapper, Definition, Manifest, Mapper, OptionDiagnostic, Request, TransformResultFiles};
use tsrs_core::{CompilerOptions, P};
use tsrs_tsoptions::{new_parsed_command_line, ParsedCommandLine};
use tsrs_vfs::{vfstest, FS};

use crate::{content_mapper_identities, read_build_info_program, BuildInfo, BuildInfoReader};

// buildinfo_contentmapper_test.go:16
fn config_with_mappers(mappers: Vec<Mapper>) -> P<ParsedCommandLine> {
    let mut config = new_parsed_command_line(P::new(CompilerOptions::default()), Vec::new(), Vec::new(), Default::default());
    config.parsed_config.content_mappers = mappers;
    P::new(config)
}

// buildinfo_contentmapper_test.go:52
struct fakeBuildInfoReader {
    build_info: BuildInfo,
}

impl BuildInfoReader for fakeBuildInfoReader {
    fn read_build_info(&self, _config: &ParsedCommandLine) -> Option<BuildInfo> {
        Some(self.build_info.clone())
    }
}

// buildinfo_contentmapper_test.go:60
struct fakeContentMapperProject {
    identities: Vec<String>,
    err: Option<contentmapper::Error>,
}

impl contentmapper::Project for fakeContentMapperProject {
    fn refresh(&self) -> Result<(), contentmapper::Error> {
        Ok(())
    }
    fn identities(&self) -> Result<Vec<String>, contentmapper::Error> {
        match &self.err {
            Some(err) => Err(err.clone()),
            None => Ok(self.identities.clone()),
        }
    }
    fn identity(&self, _mapper: &Mapper) -> Result<String, contentmapper::Error> {
        Ok(String::new())
    }
    fn watched_files(&self) -> Result<Vec<String>, contentmapper::Error> {
        Ok(Vec::new())
    }
    fn diagnostics(&self) -> Vec<OptionDiagnostic> {
        Vec::new()
    }
    fn transform(&self, _mapper: &Mapper, _request: Request<'_>) -> Result<TransformResultFiles, contentmapper::Error> {
        Ok(TransformResultFiles::default())
    }
    fn close(&self) -> Result<(), contentmapper::Error> {
        Ok(())
    }
}

fn mapper(package: &str, name: &str, version: &str, dynamic_config: bool) -> Mapper {
    Mapper {
        definition: Definition { package: package.to_string(), extensions: vec![".vue".to_string()], options: String::new() },
        manifest: Manifest { name: name.to_string(), version: version.to_string(), dynamic_config, ..Default::default() },
        ..Default::default()
    }
}

fn host_with(project: Arc<dyn contentmapper::Project>) -> Arc<dyn tsrs_compiler::CompilerHost> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(std::iter::empty::<(&str, &str)>(), true));
    tsrs_compiler::new_compiler_host("/", fs, "", None, None, Some(project))
}

fn incremental_build_info(content_mapper_identities: &[&str]) -> BuildInfo {
    BuildInfo {
        version: tsrs_core::version().to_string(),
        file_names: vec!["/src/a.ts".to_string()],
        content_mapper_identities: Some(content_mapper_identities.iter().map(|s| s.to_string()).collect()),
        ..Default::default()
    }
}

// buildinfo_contentmapper_test.go:79. A dynamic-config mapper's identity is whatever the project reports (an opaque
// configuration identity included); a change in it discards the old program.
#[test]
fn test_dynamic_content_mapper_identities() {
    let config = config_with_mappers(vec![mapper("dynamic", "dynamic", "1.0.0", true)]);
    let project = Arc::new(fakeContentMapperProject { identities: vec!["dynamic@1.0.0:opaque".to_string()], err: None });
    let identities = content_mapper_identities(Some(&*project)).unwrap();
    assert_eq!(identities, Some(project.identities.clone()));

    let build_info = incremental_build_info(&["dynamic@1.0.0:old"]);
    let host = host_with(project);
    let program = read_build_info_program(config, &fakeBuildInfoReader { build_info }, &*host);
    assert!(program.is_none(), "expected opaque mapper identity changes to discard the old program");
}

// buildinfo_contentmapper_test.go:101. The project's error is returned as it is, with no identities.
#[test]
fn test_content_mapper_identity_error() {
    let want = contentmapper::Error::Other("identity failed".to_string());
    let result = content_mapper_identities(Some(&fakeContentMapperProject { identities: Vec::new(), err: Some(want.clone()) }));
    match result {
        Ok(identities) => panic!("expected an error, got {identities:?}"),
        Err(err) => assert_eq!(err.error(), want.error()),
    }
}

// buildinfo_contentmapper_test.go:109
#[test]
fn test_read_build_info_program_content_mapper_identity_mismatch() {
    // An otherwise-valid, incremental build info whose recorded mapper identity differs from the current
    // project cannot be reused: the old program is discarded (nil) so the project is rebuilt.
    let build_info = incremental_build_info(&["vue@1.0.0"]);
    let config = config_with_mappers(vec![mapper("vue", "vue", "2.0.0", false)]);
    let project = Arc::new(fakeContentMapperProject { identities: vec!["vue@2.0.0:current".to_string()], err: None });
    let host = host_with(project);

    let program = read_build_info_program(config, &fakeBuildInfoReader { build_info }, &*host);
    assert!(program.is_none(), "expected the old program to be discarded when the mapper identity changed");
}
