import * as fs from "fs";
import * as vscode from "vscode";
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from "vscode-languageclient/node";
import { countBlockBalance, lineOpensBlock, stripCommentsAndStrings } from "./blockBalance";

let client: LanguageClient | undefined;

function serverPath(context: vscode.ExtensionContext): string {
    const configured = vscode.workspace.getConfiguration("luar").get<string>("serverPath", "");
    if (configured && configured.trim().length > 0) {
        return configured;
    }
    const binary = process.platform === "win32" ? "server/luar-lsp.exe" : "server/luar-lsp";
    return context.asAbsolutePath(binary);
}

async function startClient(context: vscode.ExtensionContext): Promise<void> {
    const path = serverPath(context);
    if (!fs.existsSync(path)) {
        void vscode.window.showWarningMessage(
            `Luar LSP server not found at ${path} — language features disabled, syntax highlighting still active.`
        );
        return;
    }
    const serverOptions: ServerOptions = {
        run: { command: path, transport: TransportKind.stdio },
        debug: { command: path, transport: TransportKind.stdio },
    };
    const config = vscode.workspace.getConfiguration("luar");
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ language: "luar" }],
        synchronize: { configurationSection: "luar" },
        initializationOptions: {
            inlayHints: config.get<boolean>("inlayHints", true),
            showMutability: config.get<boolean>("showMutability", true),
            autoImport: config.get<boolean>("autoImport", true),
        },
    };
    client = new LanguageClient("luar-lsp", "Luar Language Server", serverOptions, clientOptions);
    await client.start();
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    await startClient(context);

    context.subscriptions.push(
        vscode.commands.registerCommand("luar.restartServer", async () => {
            if (client) {
                await client.stop();
                client = undefined;
            }
            await startClient(context);
        })
    );

    context.subscriptions.push(
        vscode.workspace.onDidChangeTextDocument((event) => {
            void insertEndAfterBlockOpener(event).then(undefined, () => undefined);
        })
    );

    const aliasWatcher = vscode.workspace.createFileSystemWatcher("**/{luari.json,luari,.luari}");
    const notifyAliasChange = (uri: vscode.Uri, changeType: number) => {
        if (!client) {
            return;
        }
        void client.sendNotification("workspace/didChangeWatchedFiles", {
            changes: [{ uri: uri.toString(), type: changeType }],
        });
    };
    aliasWatcher.onDidCreate((uri) => notifyAliasChange(uri, 1));
    aliasWatcher.onDidChange((uri) => notifyAliasChange(uri, 2));
    aliasWatcher.onDidDelete((uri) => notifyAliasChange(uri, 3));
    context.subscriptions.push(aliasWatcher);
}

export async function deactivate(): Promise<void> {
    if (client) {
        await client.stop();
        client = undefined;
    }
}

async function insertEndAfterBlockOpener(event: vscode.TextDocumentChangeEvent): Promise<void> {
    const document = event.document;
    if (document.languageId !== "luar") {
        return;
    }
    if (event.contentChanges.length !== 1) {
        return;
    }
    const change = event.contentChanges[0];
    if (change.rangeLength !== 0) {
        return;
    }
    if (!/^\r?\n[ \t]*$/.test(change.text)) {
        return;
    }
    const autoIndentEnabled = vscode.workspace
        .getConfiguration("luar", document.uri)
        .get<boolean>("autoIndent", true);
    if (!autoIndentEnabled) {
        return;
    }

    const strippedLines = stripCommentsAndStrings(document.getText()).split("\n");
    const openerLineNumber = change.range.start.line;
    const openerLine = strippedLines[openerLineNumber] ?? "";
    const rawOpener = document.lineAt(openerLineNumber).text;
    const keyword = missingBlockKeyword(openerLine, rawOpener);
    const amendedOpener = keyword ? `${openerLine}${keyword}` : openerLine;
    strippedLines[openerLineNumber] = amendedOpener;

    const isElseif = /^\s*elseif\b/.test(amendedOpener);
    const wantsEnd =
        !isElseif && lineOpensBlock(amendedOpener) && countBlockBalance(strippedLines) > 0;
    if (!keyword && !wantsEnd) {
        return;
    }

    const cursorLineNumber = openerLineNumber + 1;
    if (cursorLineNumber >= document.lineCount) {
        return;
    }
    const edit = new vscode.WorkspaceEdit();
    if (keyword) {
        edit.insert(document.uri, document.lineAt(openerLineNumber).range.end, keyword);
    }
    if (wantsEnd) {
        const cursorLine = document.lineAt(cursorLineNumber);
        const trailing = cursorLine.text;
        const firstNonWs = trailing.search(/\S/);
        const closersOnly = firstNonWs >= 0 && /^[)\]},;]+$/.test(trailing.slice(firstNonWs).trimEnd());
        const insertPosition = closersOnly
            ? new vscode.Position(cursorLineNumber, firstNonWs)
            : cursorLine.range.end;
        const openerIndent = rawOpener.match(/^[ \t]*/)?.[0] ?? "";
        const eol = document.eol === vscode.EndOfLine.CRLF ? "\r\n" : "\n";
        edit.insert(document.uri, insertPosition, `${eol}${openerIndent}end`);
    }
    await vscode.workspace.applyEdit(edit);
}

function missingBlockKeyword(strippedLine: string, rawLine: string): string | null {
    const trimmed = strippedLine.trim();
    const rawTrimmed = rawLine.trimEnd();
    if (!/[\w)\]"'`]$/.test(rawTrimmed)) {
        return null;
    }
    if (/\b(and|or|not)$/.test(trimmed)) {
        return null;
    }
    if (/^(if|elseif)\b/.test(trimmed) && !/\bthen\b/.test(trimmed)) {
        return /^(if|elseif)\s*$/.test(trimmed) ? null : " then";
    }
    if (/^(for|while)\b/.test(trimmed) && !/\bdo\b/.test(trimmed)) {
        return /^(for|while)\s*$/.test(trimmed) ? null : " do";
    }
    return null;
}
