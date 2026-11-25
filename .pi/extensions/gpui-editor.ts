import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";
import { execFile } from "node:child_process";
import { randomInt } from "node:crypto";
import type { BigIntStats } from "node:fs";
import { lstat, opendir, realpath } from "node:fs/promises";
import { Socket } from "node:net";
import { isAbsolute, join, resolve } from "node:path";
import { promisify, stripVTControlCharacters } from "node:util";

// Enable Developer Mode in GPUI Editor, then run /gpui-pair and check the
// matching code in the editor before approving. /gpui-unpair revokes access.
const VERSION = 1;
const MAX_FRAME = 6 * 1024 * 1024; // Includes the terminating LF.
const MAX_ID = 0xffff_ffff;
const MAX_LOGS = 100;
const MAX_LOG_BYTES = 50 * 1024;
const MAX_LOG_LINES = 2000;
const PNG_HEADER = Buffer.from("89504e470d0a1a0a0000000d49484452", "hex");
const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
const timeouts = { discovery: 5000, connect: 500, select: 30_000, approval: 60_000, frame: 10_000, logs: 5000 };

type Frame = Record<string, unknown>;
type Endpoint = { path: string; socket: BigIntStats };
type PairRequest = { type: "pair"; version: number; project: string; session: string; code: string };
type PendingLogs = { limit: number; finish(error: Error | null, entries?: string[]): void };

function record(value: unknown): value is Frame {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function u32(value: unknown): value is number {
	return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= MAX_ID;
}

function printable(value: string): string {
	// Keep tabs and LF for plain text, but do not pass terminal control codes through.
	return stripVTControlCharacters(value).replace(/[\x00-\x08\x0b-\x1f\x7f-\x9f]/g, "");
}

function utf8Head(value: string, limit: number): string {
	const bytes = Buffer.from(value);
	if (bytes.length <= limit) return value;
	while ((bytes[limit] & 0xc0) === 0x80) limit--;
	return bytes.toString("utf8", 0, limit);
}

async function darwinUserTempDir(signal: AbortSignal): Promise<string> {
	if (process.platform !== "darwin") throw new Error("GPUI pairing requires macOS.");
	const { stdout } = await promisify(execFile)("/usr/bin/getconf", ["DARWIN_USER_TEMP_DIR"], {
		encoding: "utf8", maxBuffer: 4096, signal, timeout: 2000, shell: false,
	});
	return stdout.replace(/\r?\n$/, "");
}

async function privateStat(path: string, type: "directory" | "socket"): Promise<BigIntStats | undefined> {
	const stat = await lstat(path, { bigint: true }).catch(() => undefined);
	const uid = process.geteuid?.();
	const mode = type === "directory" ? 0o700n : 0o600n;
	if (!stat || uid === undefined || stat.uid !== BigInt(uid) || (stat.mode & 0o7777n) !== mode) return;
	if (type === "directory" ? stat.isDirectory() : stat.isSocket()) return stat;
}

async function endpoint(root: string, name: string): Promise<Endpoint | undefined> {
	const directory = join(root, name);
	const path = join(directory, "s");
	// Do not let the OS or runtime truncate a Unix socket address.
	if (Buffer.byteLength(path) >= (process.platform === "darwin" ? 104 : 108)) return;
	if (!await privateStat(directory, "directory")) return;
	const socket = await privateStat(path, "socket");
	if (socket) return { path, socket };
}

function openSocket(path: string, signal: AbortSignal, timeout: number): Promise<Socket> {
	signal.throwIfAborted();
	return new Promise((resolve, reject) => {
		const socket = new Socket();
		const timer = setTimeout(() => finish(new Error("GPUI Editor connection timed out.")), timeout);
		const abort = () => finish(new Error("GPUI Editor connection cancelled."));
		const connected = () => finish();
		const closed = () => finish(new Error("GPUI Editor closed the connection."));
		function finish(error?: Error) {
			clearTimeout(timer);
			signal.removeEventListener("abort", abort);
			socket.off("connect", connected).off("close", closed).off("error", finish);
			if (error) {
				socket.destroy();
				reject(error);
			} else {
				resolve(socket);
			}
		}
		signal.addEventListener("abort", abort, { once: true });
		socket.once("connect", connected).once("close", closed).once("error", finish);
		try {
			socket.connect({ path });
		} catch {
			finish(new Error("Could not connect to GPUI Editor."));
		}
	});
}

async function discover(temp: string, signal: AbortSignal, connectTimeout: number) {
	if (!isAbsolute(temp) || temp.includes("\0")) throw new Error("Invalid macOS user temp directory.");
	// Resolve system aliases (such as /var), but reject a symlink at the temp directory itself.
	const tempPath = resolve(temp);
	if (!await privateStat(tempPath, "directory")) throw new Error("Unsafe macOS user temp directory.");
	const base = await realpath(tempPath);
	const root = join(base, "gpui-pi");
	const instances = new Map<string, Endpoint>();
	if (!await privateStat(base, "directory") || !await privateStat(root, "directory")) return { base, root, instances };

	let scanned = 0;
	const candidates: [string, Endpoint][] = [];
	for await (const entry of await opendir(root)) {
		signal.throwIfAborted();
		if (++scanned > 256) throw new Error("Too many entries in the GPUI Editor socket directory.");
		if (!entry.isDirectory()) continue;
		const candidate = await endpoint(root, entry.name);
		if (candidate) candidates.push([entry.name, candidate]);
	}
	candidates.sort(([a], [b]) => a.localeCompare(b));
	for (let start = 0; start < candidates.length; start += 8) {
		signal.throwIfAborted();
		const batch = candidates.slice(start, start + 8);
		const reachable = await Promise.all(batch.map(async ([, candidate]) => {
			// Probe without sending a request or disclosing any Pi session data.
			const socket = await openSocket(candidate.path, signal, connectTimeout).catch(() => undefined);
			socket?.destroy();
			return socket !== undefined;
		}));
		batch.forEach(([name, candidate], index) => {
			if (reachable[index]) instances.set(name, candidate);
		});
	}
	signal.throwIfAborted();
	return { base, root, instances };
}

class EditorConnection {
	paired = false;
	private closed = false;
	private approvalTimer?: ReturnType<typeof setTimeout>;
	private frameTimer?: ReturnType<typeof setTimeout>;
	private fragment?: Buffer;
	private size = 0;
	private nextId = 1;
	private readonly pending = new Map<number, PendingLogs>();

	constructor(
		private readonly socket: Socket,
		private readonly timeout: typeof timeouts,
		private readonly onPaired: () => void,
		private readonly onFeedback: (frame: Frame) => string | undefined,
		private readonly onClose: (message: string) => void,
	) {
		socket.on("data", (chunk: Buffer) => this.read(chunk));
		socket.on("error", () => this.close("GPUI Editor socket error."));
		socket.once("close", () => this.close("GPUI Editor disconnected."));
	}

	pair(request: PairRequest) {
		this.approvalTimer = setTimeout(() => this.close("GPUI Editor approval timed out."), this.timeout.approval);
		this.write(request);
	}

	close(message: string) {
		if (this.closed) return;
		this.closed = true;
		this.paired = false;
		clearTimeout(this.approvalTimer);
		clearTimeout(this.frameTimer);
		this.fragment = undefined;
		this.socket.destroy();
		for (const request of this.pending.values()) request.finish(new Error(message));
		this.onClose(message);
	}

	logs(limit: number, signal?: AbortSignal): Promise<string[]> {
		signal?.throwIfAborted();
		if (!this.paired) throw new Error("Run /gpui-pair and approve the connection in GPUI Editor first.");
		if (this.pending.size >= 8 || this.nextId > MAX_ID) throw new Error("GPUI Editor log request limit reached.");
		const id = this.nextId++;
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => finish(new Error("GPUI Editor logs timed out.")), this.timeout.logs);
			const abort = () => finish(new Error("GPUI Editor logs cancelled."));
			const finish: PendingLogs["finish"] = (error, entries) => {
				clearTimeout(timer);
				signal?.removeEventListener("abort", abort);
				this.pending.delete(id);
				if (error) reject(error);
				else resolve(entries!);
			};
			this.pending.set(id, { limit, finish });
			signal?.addEventListener("abort", abort, { once: true });
			this.write({ type: "logs", id, limit });
		});
	}

	private write(message: Frame) {
		if (this.closed) return;
		const line = `${JSON.stringify(message)}\n`;
		// Requests and receipts are small. Close if the peer is not reading them.
		if (this.socket.destroyed || this.socket.writableLength + Buffer.byteLength(line) > 64 * 1024) {
			this.close("GPUI Editor socket is not writable.");
			return;
		}
		this.socket.write(line);
	}

	private append(bytes: Buffer) {
		const needed = this.size + bytes.length;
		if (!this.fragment || this.fragment.length < needed) {
			const buffer = Buffer.allocUnsafe(Math.min(MAX_FRAME - 1, Math.max(needed, (this.fragment?.length ?? 2048) * 2)));
			this.fragment?.copy(buffer, 0, 0, this.size);
			this.fragment = buffer;
		}
		bytes.copy(this.fragment, this.size);
		this.size = needed;
	}

	private read(chunk: Buffer) {
		try {
			for (let start = 0; start < chunk.length && !this.closed;) {
				const lf = chunk.indexOf(0x0a, start);
				const bytes = chunk.subarray(start, lf === -1 ? chunk.length : lf);
				if (this.size + bytes.length >= MAX_FRAME) throw new Error("Oversize frame");
				if (lf === -1) {
					this.frameTimer ??= setTimeout(() => this.close("GPUI Editor frame timed out."), this.timeout.frame);
					this.append(bytes);
					return;
				}
				let line = bytes;
				if (this.fragment) {
					this.append(bytes);
					line = this.fragment.subarray(0, this.size);
					this.fragment = undefined;
					this.size = 0;
				}
				clearTimeout(this.frameTimer);
				this.frameTimer = undefined;
				this.receive(JSON.parse(utf8.decode(line)));
				start = lf + 1;
			}
		} catch {
			this.close("Invalid GPUI Editor protocol frame.");
		}
	}

	private receive(frame: unknown) {
		if (!record(frame)) throw new Error("Expected a JSON object");
		if (!this.paired) {
			if (frame.type === "rejected" && typeof frame.message === "string") {
				const reason = printable(frame.message.slice(0, 240)).replace(/\s+/g, " ").trim();
				this.close(reason ? `GPUI Editor rejected pairing: ${reason}` : "GPUI Editor rejected pairing.");
				return;
			}
			if (frame.type !== "paired" || frame.version !== VERSION) throw new Error("Expected pairing approval");
			clearTimeout(this.approvalTimer);
			this.paired = true;
			this.onPaired();
			return;
		}
		if (!u32(frame.id)) throw new Error("Invalid request ID");
		if (frame.type === "feedback") {
			const message = this.onFeedback(frame);
			this.write({ type: "receipt", id: frame.id, accepted: message === undefined, message });
		} else if (frame.type === "logs") {
			const request = this.pending.get(frame.id);
			if (!request) return; // A late reply to a cancelled or expired request.
			if (!Array.isArray(frame.entries) || frame.entries.length > request.limit ||
				!frame.entries.every((entry) => typeof entry === "string")) throw new Error("Invalid log entries");
			request.finish(null, frame.entries);
		} else {
			throw new Error("Unexpected frame type");
		}
	}
}

function feedbackContent(frame: Frame): Exclude<Parameters<ExtensionAPI["sendUserMessage"]>[0], string> | undefined {
	const { target, text, image } = frame;
	if (!record(target) || typeof target.id !== "string" || typeof target.label !== "string" ||
		typeof text !== "string" || !Array.isArray(target.bounds) || target.bounds.length !== 4 ||
		!target.bounds.every((value) => typeof value === "number" && Number.isFinite(Math.fround(value)))) return;

	const content: Exclude<Parameters<ExtensionAPI["sendUserMessage"]>[0], string> = [
		{ type: "text", text: `GPUI Editor feedback for ${JSON.stringify({ id: printable(target.id), label: printable(target.label), bounds: target.bounds })}\n\n${printable(text)}` },
	];
	if (image !== undefined) {
		if (!record(image) || image.mimeType !== "image/png" || typeof image.data !== "string") return;
		const png = Buffer.from(image.data, "base64");
		if (!png.subarray(0, PNG_HEADER.length).equals(PNG_HEADER) || png.length < 33 ||
			png.toString("base64") !== image.data) return;
		content.push({ type: "image", mimeType: "image/png", data: image.data });
	}
	return content;
}

function renderLogs(entries: string[]): string {
	if (!entries.length) return "No recent GPUI Editor logs.";
	const bytes = Buffer.from(printable(entries.join("\n")));
	let start = Math.max(0, bytes.length - (MAX_LOG_BYTES - 64));
	while ((bytes[start] & 0xc0) === 0x80) start++;
	const lines = bytes.toString("utf8", start).split("\n");
	if (!start && lines.length <= MAX_LOG_LINES) return lines.join("\n");
	return `[Earlier GPUI Editor log output truncated]\n${lines.slice(1 - MAX_LOG_LINES).join("\n")}`;
}

// Optional injected discovery and deadlines keep tests isolated from the real app.
export default function gpuiEditor(
	pi: ExtensionAPI,
	getUserTempDir = darwinUserTempDir,
	timeout = timeouts,
) {
	let stopped = false;
	let attempt: AbortController | undefined;
	let editor: EditorConnection | undefined;
	let pairedSession: string | undefined;
	let feedbackSubmissions = 0;
	let lastFeedback = -Infinity;

	function disconnect(reason: string) {
		const connection = editor;
		editor = undefined;
		pairedSession = undefined;
		const discovery = attempt;
		attempt = undefined;
		discovery?.abort();
		connection?.close(reason);
	}

	pi.on("session_shutdown", () => {
		stopped = true;
		disconnect("Pi session ended.");
	});
	pi.on("agent_settled", () => { feedbackSubmissions = 0; });

	function sessionMatches(ctx: ExtensionContext, id: string): boolean {
		return !stopped && ctx.sessionManager.getSessionId() === id;
	}

	pi.registerCommand("gpui-pair", {
		description: "Pair this Pi session with a local GPUI Editor in Developer Mode",
		async handler(args, ctx) {
			if (stopped || !ctx.hasUI) return;
			if (args.trim()) {
				ctx.ui.notify("Usage: /gpui-pair", "warning");
				return;
			}
			if (attempt || editor) {
				ctx.ui.notify("GPUI Editor is paired or pairing. Use /gpui-unpair first.", "info");
				return;
			}
			const session = ctx.sessionManager.getSessionId();
			const operation = new AbortController();
			attempt = operation;
			const current = () => attempt === operation && sessionMatches(ctx, session);
			let socket: Socket | undefined;
			try {
				const signal = AbortSignal.any([operation.signal, AbortSignal.timeout(timeout.discovery)]);
				const { base, root, instances } = await discover(await getUserTempDir(signal), signal, timeout.connect);
				if (!current()) return;
				if (instances.size === 0) throw new Error("No GPUI Editor found. Enable Developer Mode in the running app.");

				const names = [...instances.keys()];
				let name = names[0];
				if (names.length > 1) {
					const choices = names.map((value, index) => `${index + 1}: ${printable(value)}`);
					const choice = await ctx.ui.select("Choose a GPUI Editor instance", choices, {
						signal: operation.signal, timeout: timeout.select,
					});
					if (!current() || choice === undefined) return;
					const index = choices.indexOf(choice);
					if (index === -1) return;
					name = names[index];
				}

				const selected = instances.get(name)!;
				const verified = await privateStat(base, "directory") && await privateStat(root, "directory") &&
					await endpoint(root, name);
				if (!verified || verified.socket.ino !== selected.socket.ino || verified.socket.dev !== selected.socket.dev) {
					throw new Error("The GPUI Editor socket changed. Run /gpui-pair again.");
				}
				if (!current()) return;
				socket = await openSocket(verified.path, operation.signal, timeout.connect);
				if (!current()) return;

				const code = String(randomInt(1_000_000)).padStart(6, "0");
				ctx.ui.notify(`GPUI Editor pairing code: ${code}. Check the code and approve in the editor. Use /gpui-unpair to cancel.`, "info");
				const request: PairRequest = {
					type: "pair", version: VERSION, project: utf8Head(ctx.cwd, 512), session: utf8Head(session, 128), code,
				};
				if (!current()) return;
				const connection = new EditorConnection(socket, timeout, () => {
					if (!sessionMatches(ctx, session)) return disconnect("Pi session changed.");
					pairedSession = session;
					ctx.ui.notify("Paired with GPUI Editor. Feedback can enter this Pi session.", "info");
				}, (frame) => {
					if (editor !== connection || !sessionMatches(ctx, session)) {
						connection.close("Pi session changed.");
						return "Pi session changed.";
					}
					try {
						const content = feedbackContent(frame);
						if (!content) return "Invalid GPUI Editor feedback.";
						const model = ctx.model;
						if (!model) return "No model is selected in Pi.";
						if (content.length > 1 && !model.input.includes("image")) return "The selected Pi model does not support images.";
						// Bound bursts before async input handling and the queue during a run.
						const idle = ctx.isIdle();
						const now = performance.now();
						if (idle && now - lastFeedback >= 1000) feedbackSubmissions = 0;
						if (feedbackSubmissions >= 4) return "Too much feedback is pending in Pi. Try again shortly.";
						pi.sendUserMessage(content, {
							expandPromptTemplates: false,
							deliverAs: idle ? undefined : "followUp",
						});
						feedbackSubmissions++;
						lastFeedback = now;
						return undefined; // Handed to Pi, not a promise of model delivery.
					} catch {
						return "Pi could not accept the feedback.";
					}
				}, (message) => {
					if (editor !== connection) return;
					editor = undefined;
					pairedSession = undefined;
					if (sessionMatches(ctx, session)) ctx.ui.notify(message, "warning");
				});
				editor = connection;
				socket = undefined; // The connection now owns the socket.
				connection.pair(request);
			} catch (error) {
				if (current()) ctx.ui.notify(error instanceof Error ? printable(error.message) : "GPUI Editor pairing failed.", "warning");
			} finally {
				socket?.destroy();
				if (attempt === operation) attempt = undefined;
			}
		},
	});

	pi.registerCommand("gpui-unpair", {
		description: "Disconnect this Pi session from GPUI Editor or cancel pairing",
		async handler(args, ctx) {
			if (stopped) return;
			if (args.trim()) {
				if (ctx.hasUI) ctx.ui.notify("Usage: /gpui-unpair", "warning");
				return;
			}
			const connected = !!(attempt || editor);
			disconnect("GPUI Editor unpaired.");
			if (ctx.hasUI) ctx.ui.notify(connected ? "GPUI Editor unpaired." : "GPUI Editor is not paired.", "info");
		},
	});

	pi.registerTool({
		name: "gpui_editor_logs",
		label: "GPUI Editor Logs",
		description: `Read recent diagnostics from the locally paired GPUI Editor. Returns up to ${MAX_LOGS} entries with output capped at 50 KiB or ${MAX_LOG_LINES} lines.`,
		annotations: { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false },
		parameters: {
			type: "object",
			properties: { limit: { type: "integer", minimum: 1, maximum: MAX_LOGS, description: "Number of recent entries (default 30)" } },
			additionalProperties: false,
		},
		async execute(_id, params, signal, _update, ctx) {
			const limit = record(params) && params.limit !== undefined ? params.limit : 30;
			if (!u32(limit) || limit < 1 || limit > MAX_LOGS) throw new Error(`Log limit must be an integer from 1 to ${MAX_LOGS}.`);
			const connection = editor;
			const session = pairedSession;
			if (!connection?.paired || session === undefined || !sessionMatches(ctx, session)) {
				throw new Error("Run /gpui-pair and approve the connection in GPUI Editor first.");
			}
			const entries = await connection.logs(limit, signal);
			signal?.throwIfAborted();
			if (editor !== connection || !sessionMatches(ctx, session)) throw new Error("GPUI Editor pairing ended.");
			return { content: [{ type: "text", text: renderLogs(entries) }], details: undefined };
		},
	});
}
