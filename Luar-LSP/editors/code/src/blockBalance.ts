const LINE_OPENER_PATTERN =
    /\b(?:if|for|while|function|switch|case|default|constructor|destructor|operator)\b|\b(?:get|set)\s+[A-Za-z_]\w*\s*\(|^\s*do\b/;

export function stripCommentsAndStrings(source: string): string {
    let out = "";
    let i = 0;
    const n = source.length;
    let mode: "code" | "line" | "block" | "quote" | "long" | "interp" = "code";
    let quote = "";
    let longLevel = 0;

    const longOpenAt = (pos: number): number => {
        if (source[pos] !== "[") return -1;
        let j = pos + 1;
        while (j < n && source[j] === "=") j++;
        return j < n && source[j] === "[" ? j - pos - 1 : -1;
    };

    while (i < n) {
        const c = source[i];
        if (mode === "code") {
            if (c === "-" && source[i + 1] === "-") {
                const lvl = longOpenAt(i + 2);
                if (lvl >= 0) {
                    mode = "block";
                    longLevel = lvl;
                    i += 4 + lvl;
                } else {
                    mode = "line";
                    i += 2;
                }
                continue;
            }
            if (c === '"' || c === "'") {
                mode = "quote";
                quote = c;
                i++;
                continue;
            }
            if (c === "`") {
                mode = "interp";
                i++;
                continue;
            }
            const lvl = longOpenAt(i);
            if (lvl >= 0) {
                mode = "long";
                longLevel = lvl;
                i += 2 + lvl;
                continue;
            }
            out += c;
            i++;
            continue;
        }
        if (c === "\n") {
            out += "\n";
            if (mode === "line") mode = "code";
            i++;
            continue;
        }
        if (mode === "quote") {
            if (c === "\\") {
                i += 2;
                continue;
            }
            if (c === quote) mode = "code";
            i++;
            continue;
        }
        if (mode === "interp") {
            if (c === "\\") {
                i += 2;
                continue;
            }
            if (c === "`") mode = "code";
            i++;
            continue;
        }
        if (mode === "block" || mode === "long") {
            if (c === "]") {
                let j = i + 1;
                let eq = 0;
                while (j < n && source[j] === "=") {
                    eq++;
                    j++;
                }
                if (eq === longLevel && j < n && source[j] === "]") {
                    mode = "code";
                    i = j + 1;
                    continue;
                }
            }
            i++;
            continue;
        }
        i++;
    }
    return out;
}

export function countLineBalance(strippedLine: string): number {
    let balance = 0;
    const words = strippedLine.matchAll(/[A-Za-z_]\w*/g);
    let prev = "";
    for (const m of words) {
        const w = m[0];
        switch (w) {
            case "if":
            case "for":
            case "while":
            case "switch":
            case "case":
            case "default":
            case "constructor":
            case "destructor":
            case "operator":
                balance++;
                break;
            case "function":
                balance++;
                break;
            case "get":
            case "set": {
                const after = strippedLine.slice((m.index ?? 0) + w.length);
                if (/^\s+[A-Za-z_]\w*\s*\(/.test(after)) balance++;
                break;
            }
            case "do":
                if (prev !== "for" && prev !== "while" && !hasEarlierLoopWord(strippedLine, m.index ?? 0)) {
                    balance++;
                }
                break;
            case "end":
                balance--;
                break;
        }
        prev = w;
    }
    return balance;
}

function hasEarlierLoopWord(line: string, before: number): boolean {
    const head = line.slice(0, before);
    return /\b(for|while)\b/.test(head);
}

export function countBlockBalance(strippedLines: string[]): number {
    let total = 0;
    let interfaceBraces = 0;
    let pendingInterface = false;
    for (const line of strippedLines) {
        if (interfaceBraces > 0 || pendingInterface) {
            const delta = braceDelta(line);
            if (pendingInterface && /\{/.test(line)) {
                pendingInterface = false;
                interfaceBraces = Math.max(delta, 1);
            } else {
                interfaceBraces += delta;
            }
            if (interfaceBraces <= 0) {
                interfaceBraces = 0;
                pendingInterface = false;
            }
            continue;
        }
        const idx = line.search(/\binterface\b/);
        if (idx >= 0) {
            const delta = braceDelta(line.slice(idx));
            if (delta > 0) {
                interfaceBraces = delta;
            } else if (!/\{[^]*\}/.test(line.slice(idx))) {
                pendingInterface = true;
            }
            continue;
        }
        total += countLineBalance(line);
    }
    return total;
}

function braceDelta(line: string): number {
    let delta = 0;
    for (const ch of line) {
        if (ch === "{") delta++;
        else if (ch === "}") delta--;
    }
    return delta;
}

export function lineOpensBlock(strippedLine: string): boolean {
    if (!LINE_OPENER_PATTERN.test(strippedLine)) {
        return false;
    }
    return countLineBalance(strippedLine) > 0;
}
