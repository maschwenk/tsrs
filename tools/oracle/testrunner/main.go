// tsrs-oracle-testrunner: ground truth for the tsrs test harness.
//
//	names                 one JSON line per test file: suite, file, variants (configuredName + skip reason)
//	diags [filter]        one JSON line per test variant: input files, diagnostics (structured), pretty flag,
//	                      and the Go-rendered error baseline text
//
// Replicates internal/testrunner/compiler_runner.go newCompilerTest (unexported) with exported pieces.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/microsoft/TypeScript/tsc/internal/ast"
	"github.com/microsoft/TypeScript/tsc/internal/core"
	"github.com/microsoft/TypeScript/tsc/internal/diagnosticwriter"
	"github.com/microsoft/TypeScript/tsc/internal/locale"
	"github.com/microsoft/TypeScript/tsc/internal/parser"
	"github.com/microsoft/TypeScript/tsc/internal/repo"
	"github.com/microsoft/TypeScript/tsc/internal/testrunner"
	"github.com/microsoft/TypeScript/tsc/internal/testutil/harnessutil"
	"github.com/microsoft/TypeScript/tsc/internal/testutil/tsbaseline"
	"github.com/microsoft/TypeScript/tsc/internal/tsoptions"
	"github.com/microsoft/TypeScript/tsc/internal/tsoptions/tsoptionstest"
	"github.com/microsoft/TypeScript/tsc/internal/tspath"
	"github.com/microsoft/TypeScript/tsc/internal/vfs/osvfs"
)

var optionRegex = regexp.MustCompile(`(?m)^\/{2}\s*@(\w+)\s*:\s*([^\r\n]*)`)
var referencesRegex = regexp.MustCompile(`reference\spath`)

const srcFolder = "/.src"

func extractCompilerSettings(content string) map[string]string {
	opts := make(map[string]string)
	for _, match := range optionRegex.FindAllStringSubmatch(content, -1) {
		opts[strings.ToLower(match[1])] = strings.TrimSuffix(strings.TrimSpace(match[2]), ";")
	}
	return opts
}

var compilerVaryBy = func() map[string]struct{} {
	varyByOptions := append(
		core.Map(core.Filter(tsoptions.OptionsDeclarations, func(option *tsoptions.CommandLineOption) bool {
			return !option.IsCommandLineOnly &&
				(option.Kind == tsoptions.CommandLineOptionTypeBoolean || option.Kind == tsoptions.CommandLineOptionTypeEnum) &&
				(option.AffectsProgramStructure ||
					option.AffectsEmit ||
					option.AffectsModuleResolution ||
					option.AffectsBindDiagnostics ||
					option.AffectsSemanticDiagnostics ||
					option.AffectsSourceFile ||
					option.AffectsDeclarationPath ||
					option.AffectsBuildInfo)
		}), func(option *tsoptions.CommandLineOption) string {
			return option.Name
		}),
		"noEmit",
		"isolatedModules",
	)
	varyByMap := make(map[string]struct{})
	for _, option := range varyByOptions {
		varyByMap[strings.ToLower(option)] = struct{}{}
	}
	return varyByMap
}()

type testUnit struct {
	content string
	name    string
}

type testCaseContent struct {
	testUnitData         []*testUnit
	tsConfig             *tsoptions.ParsedCommandLine
	tsConfigFileUnitData *testUnit
	symlinks             map[string]string
}

func makeUnitsFromTest(code string, fileName string) testCaseContent {
	testUnits, symlinks, currentDirectory, globalOptions, _ := testrunner.ParseTestFilesAndSymlinks(
		code,
		fileName,
		func(filename string, content string, fileOptions map[string]string) (*testUnit, error) {
			return &testUnit{content: content, name: filename}, nil
		},
	)
	if currentDirectory == "" {
		currentDirectory = srcFolder
	}
	allFiles := make(map[string]string)
	for _, data := range testUnits {
		allFiles[tspath.GetNormalizedAbsolutePath(data.name, currentDirectory)] = data.content
	}
	parseConfigHost := tsoptionstest.NewVFSParseConfigHostWithSymlinks(allFiles, symlinks, currentDirectory, true)
	var existingOptions *core.CompilerOptions
	if globalOptions["runexternalcode"] == "true" {
		existingOptions = &core.CompilerOptions{RunExternalCode: core.TSTrue}
	}
	var tsConfig *tsoptions.ParsedCommandLine
	var tsConfigFileUnitData *testUnit
	for i, data := range testUnits {
		if harnessutil.GetConfigNameFromFileName(data.name) != "" {
			configFileName := tspath.GetNormalizedAbsolutePath(data.name, currentDirectory)
			path := tspath.ToPath(data.name, parseConfigHost.GetCurrentDirectory(), parseConfigHost.Vfs.UseCaseSensitiveFileNames())
			configJson := parser.ParseSourceFile(ast.SourceFileParseOptions{FileName: configFileName, Path: path}, data.content, core.ScriptKindJSON)
			tsConfigSourceFile := &tsoptions.TsConfigSourceFile{SourceFile: configJson}
			configDir := tspath.GetDirectoryPath(configFileName)
			tsConfig = tsoptions.ParseJsonSourceFileConfigFileContent(tsConfigSourceFile, parseConfigHost, configDir, existingOptions, nil, configFileName, nil, nil)
			tsConfigFileUnitData = data
			testUnits = slices.Delete(testUnits, i, i+1)
			break
		}
	}
	return testCaseContent{testUnits, tsConfig, tsConfigFileUnitData, symlinks}
}

var skippedTests = []string{
	"APILibCheck.ts", "APISample_Watch.ts", "APISample_WatchWithDefaults.ts", "APISample_WatchWithOwnWatchHost.ts",
	"APISample_compile.ts", "APISample_jsdoc.ts", "APISample_linter.ts", "APISample_parseConfig.ts",
	"APISample_transform.ts", "APISample_watcher.ts",
	"preserveUnusedImports.ts", "noCrashWithVerbatimModuleSyntaxAndImportsNotUsedAsValues.ts",
	"verbatimModuleSyntaxCompat.ts", "verbatimModuleSyntaxCompat2.ts", "verbatimModuleSyntaxCompat3.ts",
	"verbatimModuleSyntaxCompat4.ts", "preserveValueImports.ts", "preserveValueImports_importsNotUsedAsValues.ts",
	"preserveValueImports_errors.ts", "preserveValueImports_mixedImports.ts", "preserveValueImports_module.ts",
	"importsNotUsedAsValues_error.ts", "alwaysStrictNoImplicitUseStrict.ts", "nonPrimitiveIndexingWithForInSupressError.ts",
	"parameterInitializerBeforeDestructuringEmit.ts", "mappedTypeUnionConstraintInferences.ts",
	"lateBoundConstraintTypeChecksCorrectly.ts", "keyofDoesntContainSymbols.ts", "noStrictGenericChecks.ts",
	"noImplicitUseStrict_umd.ts", "noImplicitUseStrict_system.ts", "noImplicitUseStrict_es6.ts",
	"noImplicitUseStrict_commonjs.ts", "noImplicitAnyIndexingSuppressed.ts", "excessPropertyErrorsSuppressed.ts",
	"moduleNoneDynamicImport.ts", "moduleNoneErrors.ts",
	"noErrorUsingImportExportModuleAugmentationInDeclarationFile1.ts",
	"noErrorUsingImportExportModuleAugmentationInDeclarationFile2.ts",
	"noErrorUsingImportExportModuleAugmentationInDeclarationFile3.ts",
	"requireOfJsonFileWithModuleEmitNone.ts", "requireOfJsonFileWithModuleNodeResolutionEmitNone.ts",
}

func skipReason(options *core.CompilerOptions) string {
	if options.Module == core.ModuleKindAMD {
		return "fatal: unsupported module kind amd"
	}
	if options.OutFile != "" {
		return "fatal: unsupported outFile"
	}
	switch options.Module {
	case core.ModuleKindUMD, core.ModuleKindSystem:
		return "module"
	}
	switch options.ModuleResolution {
	case core.ModuleResolutionKindNode10, core.ModuleResolutionKindClassic:
		return "moduleResolution"
	}
	if options.ESModuleInterop.IsFalse() {
		return "esModuleInterop=false"
	}
	if options.AllowSyntheticDefaultImports.IsFalse() {
		return "allowSyntheticDefaultImports=false"
	}
	if options.BaseUrl != "" {
		return "baseUrl"
	}
	if options.Target == core.ScriptTargetES5 {
		return "target=es5"
	}
	if options.AlwaysStrict.IsFalse() {
		return "alwaysStrict=false"
	}
	return ""
}

// runT runs f with a throwaway *testing.T, converting Fatal/Skip (runtime.Goexit) and panics into an error string.
func runT(f func(t *testing.T)) (failure string) {
	done := make(chan string, 1)
	go func() {
		t := &testing.T{}
		finished := false
		defer func() {
			if r := recover(); r != nil {
				done <- fmt.Sprintf("panic: %v", r)
				return
			}
			if !finished {
				done <- "fatal"
				return
			}
			done <- ""
		}()
		f(t)
		finished = true
	}()
	return <-done
}

type variant struct {
	Name string `json:"name"`
	Skip string `json:"skip,omitempty"`
}

type jsonDiag struct {
	File     string      `json:"file,omitempty"`
	Pos      int         `json:"pos"`
	End      int         `json:"end"`
	Code     int32       `json:"code"`
	Category int         `json:"category"`
	Source   string      `json:"source,omitempty"`
	Message  string      `json:"message"`
	Key      string      `json:"key,omitempty"`
	Args     []string    `json:"args,omitempty"`
	Chain    []*jsonDiag `json:"chain,omitempty"`
	Related  []*jsonDiag `json:"related,omitempty"`
}

func toJSON(d diagnosticwriter.Diagnostic, texts map[string]string) *jsonDiag {
	j := &jsonDiag{
		Pos:      d.Pos(),
		End:      d.End(),
		Code:     d.Code(),
		Category: int(d.Category()),
		Source:   d.Source(),
		Message:  d.Localize(locale.Default),
	}
	if ad, ok := d.(*diagnosticwriter.ASTDiagnostic); ok {
		j.Key = string(ad.Diagnostic.MessageKey())
		j.Args = ad.Diagnostic.MessageArgs()
	}
	if f := d.File(); f != nil {
		j.File = f.FileName()
		if !strings.HasPrefix(j.File, "bundled:///") {
			texts[j.File] = f.Text()
		}
	}
	for _, c := range d.MessageChain() {
		j.Chain = append(j.Chain, toJSON(c, texts))
	}
	for _, r := range d.RelatedInformation() {
		j.Related = append(j.Related, toJSON(r, texts))
	}
	return j
}

type jsonFile struct {
	Name    string `json:"name"`
	Content string `json:"content"`
}

type diagResult struct {
	Suite    string            `json:"suite"`
	Name     string            `json:"name"`
	Skip     string            `json:"skip,omitempty"`
	Error    string            `json:"error,omitempty"`
	Pretty   bool              `json:"pretty,omitempty"`
	Files    []jsonFile        `json:"files,omitempty"`
	Diags    []*jsonDiag       `json:"diags,omitempty"`
	Texts    map[string]string `json:"texts,omitempty"`
	Baseline string            `json:"baseline,omitempty"`
}

type testFile struct {
	suite string
	path  string
}

func enumerate() []testFile {
	var result []testFile
	for _, suite := range []string{"compiler", "conformance"} {
		files, err := harnessutil.EnumerateFiles("tests/cases/"+suite, regexp.MustCompile(`\.tsx?$`), true)
		if err != nil {
			panic(err)
		}
		for _, f := range files {
			result = append(result, testFile{suite, f})
		}
	}
	return result
}

func configuredNameOf(basename string, config *harnessutil.NamedTestConfiguration) string {
	if config != nil && config.Name != "" {
		extname := tspath.GetAnyExtensionFromPath(basename, nil, false)
		return fmt.Sprintf("%s(%s)%s", basename[:len(basename)-len(extname)], config.Name, extname)
	}
	return basename
}

func configurations(content string) ([]*harnessutil.NamedTestConfiguration, string) {
	var configs []*harnessutil.NamedTestConfiguration
	failure := runT(func(t *testing.T) {
		configs = harnessutil.GetFileBasedTestConfigurations(t, extractCompilerSettings(content), compilerVaryBy)
	})
	return configs, failure
}

func namesOf(tf testFile) map[string]any {
	content, _ := osvfs.FS().ReadFile(tf.path)
	basename := tspath.GetBaseFileName(tf.path)
	out := map[string]any{"suite": tf.suite, "file": basename}
	if slices.Contains(skippedTests, basename) {
		out["skipped"] = true
		return out
	}
	configs, failure := configurations(content)
	if failure != "" {
		out["error"] = failure
		return out
	}
	var variants []variant
	if len(configs) == 0 {
		configs = []*harnessutil.NamedTestConfiguration{nil}
	}
	for _, config := range configs {
		v := variant{Name: configuredNameOf(basename, config)}
		v.Skip = optionsSkip(content, tf.path, config)
		variants = append(variants, v)
	}
	out["variants"] = variants
	return out
}

func optionsSkip(content string, filename string, config *harnessutil.NamedTestConfiguration) string {
	var reason string
	failure := runT(func(t *testing.T) {
		payload := makeUnitsFromTest(content, filename)
		var options *core.CompilerOptions
		if payload.tsConfig != nil {
			options = payload.tsConfig.ParsedConfig.CompilerOptions.Clone()
		}
		if options == nil {
			options = &core.CompilerOptions{}
		}
		var cfg harnessutil.TestConfiguration
		if config != nil {
			cfg = config.Config
		}
		harnessOptions := harnessutil.HarnessOptions{}
		if cfg != nil {
			harnessutil.SetOptionsFromTestConfig(t, cfg, options, &harnessOptions, srcFolder, false)
		}
		reason = skipReason(options)
	})
	if failure != "" {
		return "error: " + failure
	}
	return reason
}

func diagsOf(tf testFile, emit func(diagResult)) {
	content, _ := osvfs.FS().ReadFile(tf.path)
	basename := tspath.GetBaseFileName(tf.path)
	if slices.Contains(skippedTests, basename) {
		return
	}
	configs, failure := configurations(content)
	if failure != "" {
		emit(diagResult{Suite: tf.suite, Name: basename, Error: failure})
		return
	}
	if len(configs) == 0 {
		configs = []*harnessutil.NamedTestConfiguration{nil}
	}
	for _, config := range configs {
		res := diagResult{Suite: tf.suite, Name: configuredNameOf(basename, config)}
		res.Error = runT(func(t *testing.T) { runVariant(t, content, tf.path, config, &res) })
		emit(res)
	}
}

func runVariant(t *testing.T, content string, filename string, namedConfiguration *harnessutil.NamedTestConfiguration, res *diagResult) {
	testContent := makeUnitsFromTest(content, filename)
	var harnessConfig harnessutil.TestConfiguration
	if namedConfiguration != nil {
		harnessConfig = namedConfiguration.Config
	}
	currentDirectory := tspath.GetNormalizedAbsolutePath(harnessConfig["currentdirectory"], srcFolder)
	units := testContent.testUnitData
	create := func(unit *testUnit) *harnessutil.TestFile {
		return &harnessutil.TestFile{UnitName: tspath.GetNormalizedAbsolutePath(unit.name, currentDirectory), Content: unit.content}
	}
	var toBeCompiled, otherFiles, tsConfigFiles []*harnessutil.TestFile
	if testContent.tsConfig != nil {
		tsConfigFiles = []*harnessutil.TestFile{create(testContent.tsConfigFileUnitData)}
		for _, unit := range units {
			if slices.Contains(testContent.tsConfig.ParsedConfig.FileNames, tspath.GetNormalizedAbsolutePath(unit.name, currentDirectory)) {
				toBeCompiled = append(toBeCompiled, create(unit))
			} else {
				otherFiles = append(otherFiles, create(unit))
			}
		}
	} else {
		if baseUrl, ok := harnessConfig["baseurl"]; ok && !tspath.IsRootedDiskPath(baseUrl) {
			harnessConfig["baseurl"] = tspath.GetNormalizedAbsolutePath(baseUrl, currentDirectory)
		}
		lastUnit := units[len(units)-1]
		if harnessConfig["noimplicitreferences"] != "" || strings.Contains(lastUnit.content, "require(") || referencesRegex.MatchString(lastUnit.content) {
			toBeCompiled = append(toBeCompiled, create(lastUnit))
			for _, unit := range units[:len(units)-1] {
				otherFiles = append(otherFiles, create(unit))
			}
		} else {
			toBeCompiled = core.Map(units, create)
		}
	}
	result := harnessutil.CompileFiles(t, toBeCompiled, otherFiles, harnessConfig, testContent.tsConfig, currentDirectory, testContent.symlinks)
	res.Skip = skipReason(result.Options)
	for _, file := range core.Concatenate(toBeCompiled, otherFiles) {
		if sf := result.Program.GetSourceFile(file.UnitName); sf != nil && sf.ContentMapper() != "" {
			file.Content = sf.Text()
		}
	}
	files := core.Concatenate(tsConfigFiles, core.Concatenate(toBeCompiled, otherFiles))
	for _, f := range files {
		res.Files = append(res.Files, jsonFile{f.UnitName, f.Content})
	}
	res.Pretty = result.Options.Pretty.IsTrue()
	res.Texts = map[string]string{}
	for _, d := range diagnosticwriter.WrapASTDiagnostics(result.Diagnostics) {
		res.Diags = append(res.Diags, toJSON(d, res.Texts))
	}
	if len(result.Diagnostics) > 0 {
		res.Baseline = tsbaseline.GetErrorBaseline(t, files, diagnosticwriter.WrapASTDiagnostics(result.Diagnostics), diagnosticwriter.CompareASTDiagnostics, res.Pretty)
	}
}

func main() {
	_ = repo.TestDataPath()
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: tsrs-oracle-testrunner names|diags [filter]")
		os.Exit(2)
	}
	var filter *regexp.Regexp
	if len(os.Args) > 2 {
		filter = regexp.MustCompile(os.Args[2])
	}
	if os.Args[1] == "options" {
		dumpOptions()
		return
	}
	tests := enumerate()
	if filter != nil {
		tests = slices.DeleteFunc(tests, func(tf testFile) bool { return !filter.MatchString(filepath.Base(tf.path)) })
	}
	var mu sync.Mutex
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	write := func(v any) {
		mu.Lock()
		defer mu.Unlock()
		_ = enc.Encode(v)
	}
	work := make(chan testFile)
	var wg sync.WaitGroup
	for range runtime.NumCPU() {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for tf := range work {
				switch os.Args[1] {
				case "names":
					write(namesOf(tf))
				case "options":
				case "diags":
					diagsOf(tf, func(r diagResult) { write(r) })
				}
			}
		}()
	}
	for _, tf := range tests {
		work <- tf
	}
	close(work)
	wg.Wait()
}

func dumpOptions() {
	type opt struct {
		Name    string      `json:"name"`
		Kind    string      `json:"kind"`
		Enum    [][2]string `json:"enum,omitempty"`
		Vary    bool        `json:"vary,omitempty"`
		File    bool        `json:"file,omitempty"`
		ElemFile bool       `json:"elemFile,omitempty"`
	}
	var out []opt
	all := core.Concatenate(tsoptions.OptionsDeclarations, []*tsoptions.CommandLineOption{
		{Name: "allowNonTsExtensions", Kind: tsoptions.CommandLineOptionTypeBoolean},
		{Name: "noErrorTruncation", Kind: tsoptions.CommandLineOptionTypeBoolean},
		{Name: "suppressOutputPathCheck", Kind: tsoptions.CommandLineOptionTypeBoolean},
		{Name: "noCheck", Kind: tsoptions.CommandLineOptionTypeBoolean},
	})
	for _, o := range all {
		j := opt{Name: o.Name, Kind: string(o.Kind), File: o.IsFilePath}
		_, j.Vary = compilerVaryBy[strings.ToLower(o.Name)]
		if o.Kind == tsoptions.CommandLineOptionTypeEnum {
			for k, v := range o.EnumMap().Entries() {
				j.Enum = append(j.Enum, [2]string{k, fmt.Sprintf("%T:%v", v, v)})
			}
		}
		if o.Kind == tsoptions.CommandLineOptionTypeList || o.Kind == tsoptions.CommandLineOptionTypeListOrElement {
			j.ElemFile = o.Elements().IsFilePath
		}
		out = append(out, j)
	}
	enc := json.NewEncoder(os.Stdout)
	_ = enc.Encode(out)
}
