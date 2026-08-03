const esbuild = require("esbuild");
const fs = require("fs");
const path = require("path");

const baseConfig = {
  entryPoints: ["./src/extension.ts"],
  bundle: true,
  outfile: "./dist/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  // Source maps are a dev aid; shipping them in the .vsix leaks full sources and
  // bloats the package. `vscode:prepublish` runs with --minify.
  sourcemap: !process.argv.includes("--minify"),
  minify: process.argv.includes("--minify"),
};

async function main() {
  // Ensure dist folder exists
  const distFolder = path.join(__dirname, "dist");
  if (!fs.existsSync(distFolder)) {
    fs.mkdirSync(distFolder, { recursive: true });
  }

  // Copy proto file to dist
  const srcProto = path.join(__dirname, "..", "proto", "freecode.proto");
  const destProto = path.join(distFolder, "freecode.proto");
  if (fs.existsSync(srcProto)) {
    fs.copyFileSync(srcProto, destProto);
    console.log("Copied proto schema to dist/");
  } else {
    console.warn("Source proto schema not found at: " + srcProto);
  }

  if (process.argv.includes("--watch")) {
    let ctx = await esbuild.context(baseConfig);
    await ctx.watch();
    console.log("watching...");
  } else {
    await esbuild.build(baseConfig);
    console.log("build complete");
    await checkWebviewJs();
  }
}

// tsc/esbuild don't validate the *string* returned by getWebviewJs(); a stray escape
// there (e.g. a raw newline inside a JS string literal) silently breaks the entire
// webview panel. Compile-check it so the build fails loudly instead of shipping dead JS.
async function checkWebviewJs() {
  const tmp = path.join(__dirname, "dist", "_webview_syntax_check.cjs");
  try {
    await esbuild.build({
      entryPoints: ["./src/webview/client.ts"],
      bundle: true,
      format: "cjs",
      platform: "node",
      outfile: tmp,
      logLevel: "silent",
    });
    const { getWebviewJs } = require(tmp);
    // Compiles the webview JS body (does NOT run it) — throws on a syntax error.
    // eslint-disable-next-line no-new-func
    new Function(getWebviewJs());
    console.log("webview JS: syntax OK");
  } catch (e) {
    console.error("webview JS SYNTAX ERROR:", e.message);
    process.exit(1);
  } finally {
    // Best-effort cleanup of the scratch bundle; a failure here must not mask the
    // build result, but it should still be visible.
    try {
      fs.unlinkSync(tmp);
    } catch (e) {
      console.warn(`could not remove ${tmp}: ${e.message}`);
    }
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
