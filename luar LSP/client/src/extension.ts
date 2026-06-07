import * as path from "path";
import * as fs from "fs";
import * as vscode from "vscode";
import { ExtensionContext, window } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext): void {
  startServer(context);
  registerAutoEnd(context);
}

function startServer(context: ExtensionContext): void {

  const exe = process.platform === "win32" ? "server.exe" : "luar-lsp-server";
  const serverPath = context.asAbsolutePath(path.join("bin", exe));

  if (!fs.existsSync(serverPath)) {
    window.showErrorMessage(`LUAR: language server not found at ${serverPath}`);
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "luar" }],
  };

  client = new LanguageClient("luar", "LUAR Language Server", serverOptions, clientOptions);
  client.start();
}

function blockAlreadyClosed(
  doc: vscode.TextDocument,
  fromLine: number,
  indent: string,
): boolean {
  for (let l = fromLine; l < doc.lineCount; l++) {
    const text = doc.lineAt(l).text;
    const trimmed = text.trim();
    if (trimmed === "") continue;
    const lineIndent = (text.match(/^[ \t]*/) || [""])[0];
    if (lineIndent.length > indent.length) continue;
    return /^(end|else|elseif)\b/.test(trimmed) || trimmed.startsWith("}");
  }
  return false;
}

function looksComplete(cond: string): boolean {
  const c = cond.trim();
  if (c === "") return false;
  if (/[=<>~+\-*/%.,(^&|]$/.test(c)) return false;
  if (/\b(and|or|not|in|then|do|if|elseif|while|for|return)$/.test(c)) return false;
  return true;
}

function registerAutoEnd(context: ExtensionContext): void {
  let busy = false;
  const sub = vscode.workspace.onDidChangeTextDocument((e) => {
    if (busy) return;
    if (e.document.languageId !== "luar") return;
    if (e.contentChanges.length !== 1) return;

    const change = e.contentChanges[0];
    if (!/^\r?\n[ \t]*$/.test(change.text)) return;

    const editor = window.activeTextEditor;
    if (!editor || editor.document !== e.document) return;

    const openerLineNo = change.range.start.line;
    const cursorLineNo = openerLineNo + 1;
    if (cursorLineNo >= e.document.lineCount) return;

    const opener = e.document.lineAt(openerLineNo).text;
    const code = opener.replace(/--.*$/, "").replace(/\s+$/, "");
    const indent = (opener.match(/^[ \t]*/) || [""])[0];

    const opts = editor.options;
    const unit = opts.insertSpaces
      ? " ".repeat(typeof opts.tabSize === "number" ? opts.tabSize : 4)
      : "\t";
    const bodyIndent = indent + unit;

    let keyword: string | null = null;
    let addEnd = false;

    const ifMatch = /^\s*(if|elseif)\b(.*)$/.exec(code);
    const loopMatch = /^\s*(while|for)\b(.*)$/.exec(code);
    const endsOpener =
      /(?:\bthen|\bdo|\belse)$/.test(code) ||

      (/\b(function|constructor|operator|get|set)\b/.test(code) && /\)\s*$/.test(code));

    const closed = blockAlreadyClosed(e.document, cursorLineNo, indent);

    if (ifMatch && !/\b(then|end)\b/.test(code) && looksComplete(ifMatch[2])) {
      keyword = "then";
      addEnd = ifMatch[1] === "if" && !closed;
    } else if (loopMatch && !/\b(do|end)\b/.test(code) && looksComplete(loopMatch[2])) {
      keyword = "do";
      addEnd = !closed;
    } else if (endsOpener && !/\bend\b/.test(code)) {
      keyword = "";
      addEnd = !closed;
    } else {
      return;
    }

    if (keyword === "" && !addEnd) return;

    busy = true;
    const cursorLine = e.document.lineAt(cursorLineNo);
    const replacement = addEnd ? bodyIndent + "\n" + indent + "end" : bodyIndent;
    editor
      .edit(
        (b) => {
          if (keyword) {
            b.insert(new vscode.Position(openerLineNo, code.length), " " + keyword);
          }
          b.replace(cursorLine.range, replacement);
        },
        { undoStopBefore: false, undoStopAfter: false },
      )
      .then(() => {
        const pos = new vscode.Position(cursorLineNo, bodyIndent.length);
        editor.selection = new vscode.Selection(pos, pos);
        busy = false;
      });
  });
  context.subscriptions.push(sub);
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
