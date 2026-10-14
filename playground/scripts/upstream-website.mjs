import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const website = path.join(root, "vendor/packages/typescriptlang-org");

export function upstreamWebsite() {
  return {
    name: "upstream-playground-host",
    enforce: "pre",
    buildStart() {
      const manifest = JSON.parse(
        readFileSync(path.join(root, "vendor/WEBSITE-MANIFEST.json"), "utf8"),
      );
      for (const [file, expected] of Object.entries(manifest)) {
        const actual = createHash("sha256")
          .update(readFileSync(path.join(website, file)))
          .digest("hex");
        if (actual !== expected)
          throw new Error(`Vendored website source differs from upstream: ${file}`);
      }
    },
    resolveId(source, importer) {
      if (source === "gatsby") return path.join(root, "src/gatsby.tsx");
      if (source.endsWith("/__generated__/allPages"))
        return path.join(root, "src/website-pages.ts");
      if (importer?.includes(website) && source.endsWith("/static/js/examples/en"))
        return path.join(root, "src/generated/examples.json");
    },
    transform(code, id) {
      if (!id.startsWith(website)) return;
      const file = path.relative(website, id);
      if (file === "src/templates/play.tsx") {
        const start = code.indexOf("  useEffect(() => {");
        const end = code.indexOf("  }, [])", start) + "  }, [])".length;
        if (start < 0 || end < start)
          throw new Error("Upstream compiler bootstrap boundary changed");
        code =
          code.slice(0, start) +
          '  useEffect(() => { void import("@tsrs/host").then(host => host.startPlayground(props.pageContext)) }, [])' +
          code.slice(end);
        code = code
          .replace(/import "reflect-metadata"\n/, "")
          .replace(/import playgroundReleases[^\n]+\n/, "")
          .replace(/import \{ getPlaygroundUrls \}[^\n]+\n/, "");
      }
      if (file === "src/components/Intl.tsx") {
        code = 'import { lang as messages } from "../copy/en/en"\n' + code;
        const start = code.indexOf("  let messages = require(");
        const end = code.indexOf("  return (", start);
        code = code.slice(0, start) + code.slice(end);
      }
      if (file === "src/components/IntlLink.tsx")
        code = code
          .replace("href={to}", "href={withPrefix(to)}")
          .replace("{ GatsbyLinkProps, Link }", "{ GatsbyLinkProps, Link, withPrefix }");
      if (file === "src/components/layout.tsx") {
        // Microsoft consent services belong to typescriptlang.org; no tracking cookies here.
        code = code
          .replace('import { CookieBanner } from "./layout/CookieBanner"', "")
          .replace("<CookieBanner {...props} />", '<div id="cookie-banner" className="openx" />');
      }
      if (file === "src/components/layout/TopNav.tsx")
        code = code
          .replace('aria-label="TypeScript Home Page"', 'aria-label="tsrs Home Page"')
          .replace(
            '<span className="hide-small">TypeScript</span>',
            '<span className="hide-small">tsrs</span>',
          )
          .replace("re(['/js/docsearch.js']", "re([withPrefix('/js/docsearch.js')]");
      if (file === "src/components/layout/SiteFooter.tsx")
        code =
          'import microsoftLogo from "../../assets/microsoft-logo.png"\n' +
          code.replace('require("../../assets/microsoft-logo.png").default', "microsoftLogo");
      if (file === "src/components/HeadSEO.tsx")
        code = code.replace('"TypeScript: %s"', '"tsrs: %s"');
      return { code, map: null };
    },
  };
}
