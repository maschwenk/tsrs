import fs from "node:fs";
import module from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Same scheme as TypeScript 7's `typescript` package: the native binary ships in a per-platform package
// `<this package's name>-<process.platform>-<process.arch>`, listed in `optionalDependencies` so the package
// manager installs only the one whose `os`/`cpu` match.
export default function getExePath() {
    const override = process.env.TSRS_BINARY;
    if (override) {
        if (!fs.existsSync(override)) {
            throw new Error(`TSRS_BINARY is set to ${override}, which does not exist.`);
        }
        return override;
    }

    const __dirname = path.dirname(fileURLToPath(import.meta.url));
    const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, "..", "package.json"), "utf8"));
    const platformPackageName = `${pkg.name}-${process.platform}-${process.arch}`;
    const exeName = process.platform === "win32" ? "tsrs.exe" : "tsrs";

    let packageDir;
    try {
        const require = module.createRequire(import.meta.url);
        packageDir = path.dirname(require.resolve(`${platformPackageName}/package.json`));
    }
    catch {
        const supported = Object.keys(pkg.optionalDependencies ?? {});
        const isSupported = supported.includes(platformPackageName);
        throw new Error(
            [
                `Cannot find the optional dependency ${platformPackageName}, which contains the tsrs binary for ${process.platform}-${process.arch}.`,
                isSupported
                    ? `It is listed in ${pkg.name}'s optionalDependencies but was not installed. Common causes: installing with --no-optional / --omit=optional, ` +
                        `a lockfile or node_modules created on another platform, or pnpm \`supportedArchitectures\` excluding this platform. Reinstall to fix.`
                    : `This platform is not supported. Supported: ${supported.map(n => n.slice(pkg.name.length + 1)).join(", ")}.`,
                `To use a locally built binary instead, set TSRS_BINARY=/path/to/${exeName}.`,
            ].join("\n"),
        );
    }

    let exe = path.join(packageDir, exeName);
    if (process.platform === "win32" && exe.length >= 248) {
        exe = "\\\\?\\" + exe;
    }
    if (!fs.existsSync(exe)) {
        throw new Error(`${platformPackageName} is installed at ${packageDir} but has no ${exeName}.`);
    }
    return exe;
}
