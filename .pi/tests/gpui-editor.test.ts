// Run from the repository root: bun test ./.pi/tests/gpui-editor.test.ts
import { afterEach, expect, test } from "bun:test";
import { once } from "node:events";
import { chmod, lstat, mkdir, mkdtemp, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import gpuiEditor from "../extensions/gpui-editor.ts";

const MAX_FRAME = 6 * 1024 * 1024;
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jE1sAAAAASUVORK5CYII=";
const DEADLINES = { discovery: 2000, connect: 250, select: 1000, approval: 1500, frame: 1500, logs: 1500 };
const paired = { type: "paired", version: 1 };
const target = { id: "save", label: "Save", bounds: [2.5, 4, 20, 10] };
const feedback = { type: "feedback", id: 17, target, text: "/reload 🧪\n!not a shell command" };
const image = { mimeType: "image/png", data: PNG };
const cleanups: (() => Promise<unknown>)[] = [];

afterEach(async () => {
	while (cleanups.length) await cleanups.pop()!();
});

function channel<T>() {
	const queue: T[] = [];
	const waiting: ((value: T) => void)[] = [];
	return {
		put(value: T) {
			const take = waiting.shift();
			if (take) take(value);
			else queue.push(value);
		},
		take(): Promise<T> {
			return queue.length ? Promise.resolve(queue.shift()!) : new Promise((resolve) => waiting.push(resolve));
		},
	};
}

type Message = { type: string; id?: number; code?: string; limit?: number };
const line = (message: object) => JSON.stringify(message) + "\n";

async function fakeEditor(root: string, name = "instance-a") {
	const directory = join(root, name);
	await mkdir(directory, { mode: 0o700 });
	const path = join(directory, "s");
	const sockets = new Set<Socket>();
	const requests = channel<{ frame: Message; peer: Socket; closed: Promise<unknown> }>();
	let probes = 0;
	const server = createServer((peer) => {
		sockets.add(peer);
		let buf = "";
		let sent = false;
		const closed = new Promise((resolve) => peer.once("close", resolve));
		peer.on("error", () => undefined);
		peer.on("close", () => {
			sockets.delete(peer);
			if (!sent) probes++;
		});
		peer.setEncoding("utf8");
		peer.on("data", (chunk) => {
			buf += chunk;
			for (let i = buf.indexOf("\n"); i !== -1; i = buf.indexOf("\n")) {
				const frame = JSON.parse(buf.slice(0, i)) as Message;
				buf = buf.slice(i + 1);
				sent = true;
				requests.put({ frame, peer, closed });
			}
		});
	});
	async function stop() {
		for (const peer of sockets) peer.destroy();
		if (server.listening) await new Promise<void>((resolve) => server.close(() => resolve()));
	}
	server.listen(path);
	cleanups.push(stop);
	await once(server, "listening");
	await chmod(path, 0o600);
	return { directory, path, requests, stop, get probes() { return probes; } };
}

async function fixture(overrides: Partial<typeof DEADLINES> = {}) {
	const temp = await mkdtemp(join(await realpath(tmpdir()), "gp-"));
	cleanups.push(() => rm(temp, { recursive: true, force: true }));
	const root = join(temp, "gpui-pi");
	await mkdir(root, { mode: 0o700 });
	const notices = channel<string>();
	const notifications: string[] = [];
	const messages = channel<unknown[]>();
	const sent: unknown[][] = [];
	const selections = channel<{ choices: string[]; signal: AbortSignal }>();
	const answers = channel<string | undefined>();
	const commands = new Map<string, { handler(args: string, ctx: unknown): Promise<void> }>();
	const events = new Map<string, (event: unknown, ctx: unknown) => void>();
	type LogsResult = { content: { type: string; text: string }[] };
	let tool!: {
		name: string;
		annotations: { readOnlyHint: boolean };
		execute(id: string, params: object, signal: AbortSignal | undefined, update: undefined, ctx: unknown): Promise<LogsResult>;
	};
	let sessionId = "pi-test";
	let discoveryCalls = 0;
	const ctx = {
		hasUI: true,
		cwd: "/example/project",
		idle: true,
		model: { input: ["text", "image"] } as { input: string[] } | undefined,
		isIdle: () => ctx.idle,
		sessionManager: { getSessionId: () => sessionId },
		ui: {
			notify(text: string) { notifications.push(text); notices.put(text); },
			select(_title: string, choices: string[], { signal }: { signal: AbortSignal }) {
				selections.put({ choices, signal });
				return Promise.race([
					answers.take(),
					new Promise<undefined>((resolve) => signal.addEventListener("abort", () => resolve(undefined), { once: true })),
				]);
			},
		},
	};
	const pi = {
		on(name: string, handler: (event: unknown, ctx: unknown) => void) { events.set(name, handler); },
		registerCommand(name: string, command: { handler(args: string, ctx: unknown): Promise<void> }) { commands.set(name, command); },
		registerTool(value: typeof tool) { tool = value; },
		sendUserMessage(...args: unknown[]) { sent.push(args); messages.put(args); },
	};
	gpuiEditor(pi as unknown as Parameters<typeof gpuiEditor>[0], async () => {
		discoveryCalls++;
		return temp;
	}, { ...DEADLINES, ...overrides });
	const shutdown = (reason = "quit") => events.get("session_shutdown")!({ reason }, ctx);
	cleanups.push(async () => shutdown());
	return {
		temp, root, ctx, messages, sent, selections, answers, notices, notifications, shutdown, tool,
		get discoveryCalls() { return discoveryCalls; },
		changeSession: (id: string) => { sessionId = id; },
		pair: () => commands.get("gpui-pair")!.handler("", ctx),
		unpair: () => commands.get("gpui-unpair")!.handler("", ctx),
		logs: (params = {}, signal?: AbortSignal) => tool.execute("tool-call", params, signal, undefined, ctx),
	};
}

async function approve(f: Awaited<ReturnType<typeof fixture>>, app: Awaited<ReturnType<typeof fakeEditor>>) {
	await f.pair();
	const request = await app.requests.take();
	await f.notices.take();
	request.peer.write(line(paired));
	expect(await f.notices.take()).toMatch(/^Paired/);
	return request;
}

test("shows the code before the pair request and delivers idle and busy feedback as user messages", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	await expect(f.logs()).rejects.toThrow("approve");
	const pair = f.pair();
	const { frame, peer } = await app.requests.take();
	expect(frame).toEqual({ type: "pair", version: 1, project: f.ctx.cwd, session: "pi-test", code: expect.stringMatching(/^\d{6}$/) });
	expect(f.notifications[0]).toContain(`pairing code: ${frame.code}`);
	await pair;
	await expect(f.logs()).rejects.toThrow("approve");

	// Combine two frames and split the second frame in the middle of a UTF-8 character.
	const bytes = Buffer.from(line(paired) + line({ ...feedback, image }));
	const split = bytes.indexOf(Buffer.from("🧪")) + 2;
	peer.write(bytes.subarray(0, split));
	await f.notices.take(); // code
	expect(await f.notices.take()).toMatch(/^Paired/);
	peer.write(bytes.subarray(split));
	const [idleContent, idleOptions] = await f.messages.take();
	expect(idleContent).toEqual([
		{ type: "text", text: `GPUI Editor feedback for ${JSON.stringify(target)}\n\n${feedback.text}` },
		{ type: "image", ...image },
	]);
	expect(idleOptions).toEqual({ expandPromptTemplates: false, deliverAs: undefined });
	expect((await app.requests.take()).frame).toEqual({ type: "receipt", id: 17, accepted: true });

	f.ctx.idle = false;
	peer.write(line({ ...feedback, id: 0xffff_ffff }));
	expect((await f.messages.take())[1]).toEqual({ expandPromptTemplates: false, deliverAs: "followUp" });
	expect((await app.requests.take()).frame).toEqual({ type: "receipt", id: 0xffff_ffff, accepted: true });
});

test("skips stale or unsafe entries and asks which live instance to pair", async () => {
	const f = await fixture();
	const first = await fakeEditor(f.root, "a");
	const second = await fakeEditor(f.root, "b");
	const publicSocket = await fakeEditor(f.root, "c");
	await chmod(publicSocket.path, 0o666);
	const publicDirectory = await fakeEditor(f.root, "d");
	await chmod(publicDirectory.directory, 0o755);
	await symlink(first.directory, join(f.root, "e"));
	await mkdir(join(f.root, "f"), { mode: 0o700 });
	await symlink(first.path, join(f.root, "f", "s"));
	await mkdir(join(f.root, "g"), { mode: 0o700 });
	await writeFile(join(f.root, "g", "s"), "not a socket", { mode: 0o600 });
	const stale = await fakeEditor(f.root, "h");
	await rename(stale.path, join(stale.directory, "stale"));
	await stale.stop();
	await rename(join(stale.directory, "stale"), stale.path);
	expect((await lstat(stale.path)).isSocket()).toBe(true);

	const pair = f.pair();
	const { choices } = await f.selections.take();
	expect(choices).toEqual(["1: a", "2: b"]);
	f.answers.put(choices[1]);
	await pair;
	expect((await second.requests.take()).frame.type).toBe("pair");
	expect(first.probes).toBe(1);
	expect(publicSocket.probes).toBe(0);
	expect(publicDirectory.probes).toBe(0);
});

test("rejects a symlink socket root and skips discovery with no Pi UI", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	await rename(f.root, join(f.temp, "alias-target"));
	await symlink(join(f.temp, "alias-target"), f.root);
	f.ctx.hasUI = false;
	await f.pair();
	expect(f.discoveryCalls).toBe(0);
	f.ctx.hasUI = true;
	await f.pair();
	expect(await f.notices.take()).toContain("No GPUI Editor found");
	expect(app.probes).toBe(0);
});

test("does not send a pair request if the chosen socket is replaced or made public", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root, "a");
	await fakeEditor(f.root, "b");
	const pair = f.pair();
	const { choices } = await f.selections.take();
	await chmod(app.path, 0o666);
	f.answers.put(choices[0]);
	await pair;
	expect(await f.notices.take()).toContain("socket changed");
});

test("shutting down during selection aborts the dialog and cannot pair the old session", async () => {
	const f = await fixture();
	await fakeEditor(f.root, "a");
	await fakeEditor(f.root, "b");
	const pair = f.pair();
	const { choices, signal } = await f.selections.take();
	f.shutdown("new");
	f.answers.put(choices[0]);
	await pair;
	expect(signal.aborted).toBe(true);
	expect(f.notifications).toEqual([]);
	await f.pair();
	expect(f.discoveryCalls).toBe(1);
});

test.each([
	["app rejection", { type: "rejected", message: "User declined" }, "User declined"],
	["bad version", { type: "paired", version: 2 }, "Invalid GPUI Editor protocol"],
	["feedback before approval", feedback, "Invalid GPUI Editor protocol"],
])("closes on %s without delivering feedback", async (_name, reply, message) => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	await f.pair();
	const { peer, closed } = await app.requests.take();
	await f.notices.take();
	peer.write(line(reply));
	expect(await f.notices.take()).toContain(message);
	await closed;
	expect(f.sent).toEqual([]);
	await expect(f.logs()).rejects.toThrow("approve");
});

test("bounds pair identity strings in UTF-8 bytes and lets the user cancel before approval", async () => {
	const f = await fixture();
	f.ctx.cwd = "/" + "🧪".repeat(200);
	f.changeSession("x" + "🧪".repeat(50));
	const app = await fakeEditor(f.root);
	await f.pair();
	const { frame, closed } = await app.requests.take();
	expect(frame).toMatchObject({ project: "/" + "🧪".repeat(127), session: "x" + "🧪".repeat(31) });
	await f.unpair();
	await closed;
	await expect(f.logs()).rejects.toThrow("approve");
	expect(f.sent).toEqual([]);
});

test("times out waiting for approval without leaving the socket or tool active", async () => {
	const f = await fixture({ approval: 80 });
	const app = await fakeEditor(f.root);
	await f.pair();
	const { closed } = await app.requests.take();
	await f.notices.take();
	expect(await f.notices.take()).toContain("approval timed out");
	await closed;
	await expect(f.logs()).rejects.toThrow("approve");
});

test("sends negative receipts for missing models, unsupported images, and invalid feedback", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	const { peer } = await approve(f, app);
	for (const [model, frame, message] of [
		[undefined, feedback, "No model"],
		[{ input: ["text"] }, { ...feedback, image }, "does not support images"],
		[{ input: ["text", "image"] }, { ...feedback, image: { ...image, data: PNG + "???" } }, "Invalid"],
		[{ input: ["text"] }, { ...feedback, target: { ...target, bounds: [0, 1e90, 0, 0] } }, "Invalid"],
	] as const) {
		f.ctx.model = model && { input: [...model.input] };
		peer.write(line(frame));
		expect((await app.requests.take()).frame).toEqual({ type: "receipt", id: 17, accepted: false, message: expect.stringContaining(message) });
	}
	expect(f.sent).toEqual([]);
});

test("correlates read-only log replies, bounds output, and ignores cancelled and timed-out replies", async () => {
	const f = await fixture({ logs: 120 });
	const app = await fakeEditor(f.root);
	const { peer } = await approve(f, app);
	expect(f.tool.name).toBe("gpui_editor_logs");
	expect(f.tool.annotations.readOnlyHint).toBe(true);
	await expect(f.logs({ limit: 101 })).rejects.toThrow("integer");

	const a = f.logs({ limit: 2 });
	const b = f.logs();
	const first = (await app.requests.take()).frame;
	const second = (await app.requests.take()).frame;
	expect(first).toMatchObject({ type: "logs", limit: 2 });
	expect(second).toMatchObject({ type: "logs", limit: 30 });
	expect(first.id).not.toBe(second.id);
	peer.write(line({ type: "logs", id: second.id, entries: ["big".repeat(20_000), "tail"] }) +
		line({ type: "logs", id: first.id, entries: ["first", "second"] }));
	expect((await a).content).toEqual([{ type: "text", text: "first\nsecond" }]);
	const output = (await b).content[0].text;
	expect(Buffer.byteLength(output)).toBeLessThanOrEqual(50 * 1024);
	expect(output).toContain("truncated");
	expect(output.endsWith("\ntail")).toBe(true);

	const abort = new AbortController();
	const aRequest = f.logs({}, abort.signal);
	const cancelled = (await app.requests.take()).frame.id;
	abort.abort();
	await expect(aRequest).rejects.toThrow("cancelled");
	const bRequest = f.logs();
	const timedOut = (await app.requests.take()).frame.id;
	await expect(bRequest).rejects.toThrow("timed out");
	const c = f.logs();
	const validId = (await app.requests.take()).frame.id;
	peer.write([cancelled, timedOut, validId].map((id) => line({ type: "logs", id, entries: [String(id)] })).join(""));
	expect((await c).content[0].text).toBe(String(validId));
});

test("rejects a malformed log reply and revokes the pairing", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	const { peer, closed } = await approve(f, app);
	const logs = f.logs().catch((error: unknown) => error);
	const { frame } = await app.requests.take();
	peer.write(line({ type: "logs", id: frame.id, entries: [123] }));
	expect(await logs).toEqual(expect.objectContaining({ message: expect.stringContaining("Invalid GPUI Editor protocol") }));
	await closed;
	await expect(f.logs()).rejects.toThrow("approve");
});

test.each([
	["unpair", "unpaired"], ["reload", "session ended"], ["new", "session ended"], ["remote close", "disconnected"],
])("%s closes pairing and rejects pending logs", async (action, reason) => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	const { peer, closed } = await approve(f, app);
	const logs = f.logs().catch((error: unknown) => error);
	await app.requests.take();
	if (action === "unpair") await f.unpair();
	else if (action === "remote close") peer.end();
	else f.shutdown(action);
	expect(await logs).toEqual(expect.objectContaining({ message: expect.stringContaining(reason) }));
	await closed;
	await expect(f.logs()).rejects.toThrow("approve");
	expect(f.sent).toEqual([]);
});

test("refuses an old-session feedback frame even if shutdown has not run", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	const { peer, closed } = await approve(f, app);
	f.changeSession("replacement");
	await expect(f.logs()).rejects.toThrow("approve");
	peer.write(line(feedback));
	await closed;
	expect(f.sent).toEqual([]);
});

test("accepts exactly 6 MiB including LF and disconnects one byte over the limit", async () => {
	const f = await fixture();
	const app = await fakeEditor(f.root);
	const { peer, closed } = await approve(f, app);
	const json = Buffer.from(line(feedback));
	peer.write(Buffer.concat([Buffer.alloc(MAX_FRAME - json.length, 32), json]));
	expect((await app.requests.take()).frame).toEqual({ type: "receipt", id: 17, accepted: true });
	peer.write(Buffer.concat([Buffer.alloc(MAX_FRAME - json.length + 1, 32), json]));
	expect(await f.notices.take()).toContain("Invalid GPUI Editor protocol");
	await closed;
	expect(f.sent).toHaveLength(1);
});

test.each([
	["invalid UTF-8", Buffer.from([0xff, 0x0a]), "Invalid GPUI Editor protocol"],
	["invalid JSON", Buffer.from("{oops}\n"), "Invalid GPUI Editor protocol"],
	["invalid ID", Buffer.from(line({ ...feedback, id: -1 })), "Invalid GPUI Editor protocol"],
	["unfinished line", Buffer.from("{"), "frame timed out"],
])("disconnects on %s", async (_name, data, reason) => {
	const f = await fixture({ frame: 100 });
	const app = await fakeEditor(f.root);
	const { peer, closed } = await approve(f, app);
	peer.write(data);
	expect(await f.notices.take()).toContain(reason);
	await closed;
	expect(f.sent).toEqual([]);
});
