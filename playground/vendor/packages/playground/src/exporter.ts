// Local adaptation: use static metadata and the tsrs host serializer.
import { jsonCompilerOptions } from "../../../../src/project"
import { UI } from "./createUI"

type Sandbox = import("@typescript/sandbox").Sandbox
type CompilerOptions = import("monaco-editor").languages.typescript.CompilerOptions

export const createExporter = (sandbox: Sandbox, monaco: typeof import("monaco-editor"), ui: UI) => {
  function getJsxEmitText(option: any) {
    if (!option) return undefined
    if (typeof option === "string") return option.toLowerCase()
    return monaco.languages.typescript.JsxEmit[option].toLowerCase()
  }

  const getValidCompilerOptions = jsonCompilerOptions

  // Based on https://github.com/stackblitz/core/blob/master/sdk/src/generate.ts
  function createHiddenInput(name: string, value: string) {
    const input = document.createElement("input")
    input.type = "hidden"
    input.name = name
    input.value = value
    return input
  }

  function createProjectForm(project: any) {
    const form = document.createElement("form")

    form.method = "POST"
    form.setAttribute("style", "display:none;")

    form.appendChild(createHiddenInput("project[title]", project.title))
    form.appendChild(createHiddenInput("project[description]", project.description))
    form.appendChild(createHiddenInput("project[template]", project.template))

    if (project.tags) {
      project.tags.forEach((tag: string) => {
        form.appendChild(createHiddenInput("project[tags][]", tag))
      })
    }

    if (project.dependencies) {
      form.appendChild(createHiddenInput("project[dependencies]", JSON.stringify(project.dependencies)))
    }

    if (project.settings) {
      form.appendChild(createHiddenInput("project[settings]", JSON.stringify(project.settings)))
    }

    Object.keys(project.files).forEach(path => {
      form.appendChild(createHiddenInput(`project[files][${path}]`, project.files[path]))
    })

    return form
  }

  const typescriptVersion = (sandbox.ts as any).npmVersion || sandbox.ts.version
  // prettier-ignore
  const stringifiedCompilerOptions = () => JSON.stringify({ compilerOptions: getValidCompilerOptions(sandbox.getCompilerOptions()) }, null, '  ')

  // TODO: pull deps
  function openProjectInStackBlitz() {
    const project = {
      title: "Playground Export - ",
      description: "123",
      template: "typescript",
      files: {
        "index.ts": sandbox.getText(),
        "tsconfig.json": stringifiedCompilerOptions(),
      },
      dependencies: {
        typescript: typescriptVersion,
      },
    }
    const form = createProjectForm(project)
    form.action = "https://stackblitz.com/run?view=editor"
    // https://github.com/stackblitz/core/blob/master/sdk/src/helpers.ts#L9
    // + buildProjectQuery(options);
    form.target = "_blank"

    document.body.appendChild(form)
    form.submit()
    document.body.removeChild(form)
  }

  function openInBugWorkbench() {
    const hash = `#code/${sandbox.lzstring.compressToEncodedURIComponent(sandbox.getText())}`
    document.location.assign(`/dev/bug-workbench/${hash}`)
  }

  function openInTSAST() {
    const hash = `#code/${sandbox.lzstring.compressToEncodedURIComponent(sandbox.getText())}`
    document.location.assign(`https://ts-ast-viewer.com/${hash}`)
  }

  function openInVSCodeDev() {
    const search = document.location.search
    const hash = `#code/${sandbox.lzstring.compressToEncodedURIComponent(sandbox.getText())}`
    document.location.assign(`https://insiders.vscode.dev/tsplay/${search}${hash}`)
  }

  function openProjectInCodeSandbox() {
    const files = {
      "package.json": {
        content: {
          name: "TypeScript Playground Export",
          version: "0.0.0",
          description: "TypeScript playground exported Sandbox",
          dependencies: {
            typescript: typescriptVersion,
          },
        },
      },
      "index.ts": {
        content: sandbox.getText(),
      },
      "tsconfig.json": {
        content: stringifiedCompilerOptions(),
      },
    }

    // Using the v1 get API
    const parameters = sandbox.lzstring
      .compressToBase64(JSON.stringify({ files }))
      .replace(/\+/g, "-") // Convert '+' to '-'
      .replace(/\//g, "_") // Convert '/' to '_'
      .replace(/=+$/, "") // Remove ending '='

    const url = `https://codesandbox.io/api/v1/sandboxes/define?view=editor&parameters=${parameters}`
    document.location.assign(url)

    // Alternative using the http URL API, which uses POST. This has the trade-off where
    // the async nature of the call means that the redirect at the end triggers
    // popup security mechanisms in browsers because the function isn't blessed as
    // being a direct result of a user action.

    // fetch("https://codesandbox.io/api/v1/sandboxes/define?json=1", {
    //   method: "POST",
    //   body: JSON.stringify({ files }),
    //   headers: {
    //     Accept: "application/json",
    //     "Content-Type": "application/json"
    //   }
    // })
    // .then(x => x.json())
    // .then(data => {
    //   window.open('https://codesandbox.io/s/' + data.sandbox_id, '_blank');
    // });
  }

  function codify(code: string, ext: string) {
    return "```" + ext + "\n" + code + "\n```\n"
  }

  async function makeMarkdown() {
    const query = sandbox.createURLQueryWithCompilerOptions(sandbox)
    const fullURL = `${document.location.protocol}//${document.location.host}${document.location.pathname}${query}`
    const jsSection =
      sandbox.config.filetype === "js"
        ? ""
        : `
<details><summary><b>Output</b></summary>

${codify(await sandbox.getRunnableJS(), "ts")}

</details>
`

    return `
${codify(sandbox.getText(), "ts")}

${jsSection}

<details><summary><b>Compiler Options</b></summary>

${codify(stringifiedCompilerOptions(), "json")}

</details>

**Playground Link:** [Provided](${fullURL})
      `
  }
  async function copyAsMarkdownIssue(e: React.MouseEvent) {
    e.persist()

    const markdown = await makeMarkdown()
    ui.showModal(
      markdown,
      document.getElementById("exports-dropdown")!,
      "Markdown Version of Playground Code for GitHub Issue",
      undefined,
      e
    )
    return false
  }

  function copyForChat(e: React.MouseEvent) {
    const query = sandbox.createURLQueryWithCompilerOptions(sandbox)
    const fullURL = `${document.location.protocol}//${document.location.host}${document.location.pathname}${query}`
    const chat = `[Playground Link](${fullURL})`
    ui.showModal(chat, document.getElementById("exports-dropdown")!, "Markdown for chat", undefined, e)
    return false
  }

  function copyForChatWithPreview(e: React.MouseEvent) {
    e.persist()

    const query = sandbox.createURLQueryWithCompilerOptions(sandbox)
    const fullURL = `${document.location.protocol}//${document.location.host}${document.location.pathname}${query}`

    const ts = sandbox.getText()
    const preview = ts.length > 200 ? ts.substring(0, 200) + "..." : ts.substring(0, 200)

    const jsx = getJsxEmitText(sandbox.getCompilerOptions().jsx)
    const codeLanguage = jsx !== undefined ? "tsx" : "ts"
    const code = "```" + codeLanguage + "\n" + preview + "\n```\n"
    const chat = `${code}\n[Playground Link](${fullURL})`
    ui.showModal(chat, document.getElementById("exports-dropdown")!, "Markdown code", undefined, e)
    return false
  }

  function exportAsTweet() {
    const query = sandbox.createURLQueryWithCompilerOptions(sandbox)
    const fullURL = `${document.location.protocol}//${document.location.host}${document.location.pathname}${query}`

    document.location.assign(`http://www.twitter.com/share?url=${fullURL}`)
  }

  return {
    openProjectInStackBlitz,
    openProjectInCodeSandbox,
    copyAsMarkdownIssue,
    copyForChat,
    copyForChatWithPreview,
    openInTSAST,
    openInBugWorkbench,
    openInVSCodeDev,
    exportAsTweet,
  }
}
