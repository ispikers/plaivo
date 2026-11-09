import { Database } from "bun:sqlite";
import { expect, test } from "bun:test";
import {
	type Config,
	configFrom,
	type ExchangeError,
	exchangeError,
	type Limits,
	Relay,
	type Sql,
	TOKEN,
	type TokenRequest,
	type Tokens,
} from "./relay";

const tokens = (): Tokens => ({
	access_token: "access",
	refresh_token: "refresh",
	expires_in: 3600,
	token_type: "Bearer",
});

const config = (): Config => ({
	publicUrl: "https://relay.test",
	mal: { id: "mal-id", secret: "mal-secret" },
	anilist: { id: "ani-id", secret: "ani-secret" },
});

function memory(): Sql {
	const db = new Database(":memory:");
	return {
		exec(query, ...bindings) {
			const rows = db.query(query).all(...bindings) as Record<
				string,
				unknown
			>[];
			return { toArray: () => rows };
		},
	};
}

function fake(
	reply: () => Tokens | ExchangeError,
	overrides: { config?: Config; limits?: Limits } = {},
) {
	const seen: TokenRequest[] = [];
	const relay = new Relay(
		memory(),
		overrides.config ?? config(),
		async (request) => {
			seen.push(request);
			return reply();
		},
		overrides.limits,
	);
	return { relay, seen };
}

async function start(relay: Relay, service: string) {
	const reply = await relay.handle(
		"POST",
		"/auth/start",
		JSON.stringify({ service }),
	);
	expect(reply.status).toBe(200);
	const body = JSON.parse(reply.body);
	return {
		session: body.session as string,
		params: new URL(body.authorize_url).searchParams,
	};
}

const result = (relay: Relay, session: string) =>
	relay.handle("GET", `/auth/result?session=${session}`, "");
const callback = (relay: Relay, path: string) => relay.handle("GET", path, "");

test("mal flow delivers tokens once", async () => {
	const { relay, seen } = fake(tokens);
	const { session, params } = await start(relay, "mal");
	expect(params.get("client_id")).toBe("mal-id");
	expect(params.get("redirect_uri")).toBe(
		"https://relay.test/auth/mal/callback",
	);
	expect(params.get("code_challenge_method")).toBe("plain");
	expect((await result(relay, session)).status).toBe(204);

	const page = await callback(
		relay,
		`/auth/mal/callback?code=abc&state=${params.get("state")}`,
	);
	expect(page.status).toBe(200);
	expect(page.contentType).toBe("text/html; charset=utf-8");

	const got = await result(relay, session);
	expect(got.status).toBe(200);
	expect(JSON.parse(got.body)).toEqual(tokens());
	expect((await result(relay, session)).status).toBe(410);

	expect(seen[0].url).toBe(TOKEN.mal);
	expect("form" in seen[0] && seen[0].form).toMatchObject({
		code: "abc",
		code_verifier: params.get("code_challenge")!,
		client_secret: "mal-secret",
		redirect_uri: params.get("redirect_uri")!,
	});
});

test("anilist flow uses client secret", async () => {
	const { relay, seen } = fake(tokens);
	const { session, params } = await start(relay, "anilist");
	expect(params.has("code_challenge")).toBe(false);
	expect(
		(
			await callback(
				relay,
				`/auth/anilist/callback?code=xyz&state=${params.get("state")}`,
			)
		).status,
	).toBe(200);
	expect((await result(relay, session)).status).toBe(200);
	expect(seen[0].url).toBe(TOKEN.anilist);
	expect("json" in seen[0] && seen[0].json).toMatchObject({
		client_secret: "ani-secret",
		code: "xyz",
		grant_type: "authorization_code",
	});
});

test("unknown or reused state is rejected", async () => {
	const { relay, seen } = fake(tokens);
	const { session, params } = await start(relay, "mal");
	const state = params.get("state");
	expect(
		(await callback(relay, "/auth/mal/callback?code=abc&state=nope"))
			.status,
	).toBe(400);
	expect((await callback(relay, "/auth/mal/callback?code=abc")).status).toBe(
		400,
	);
	expect(
		(
			await callback(
				relay,
				`/auth/anilist/callback?code=abc&state=${state}`,
			)
		).status,
	).toBe(400);
	expect(
		(await callback(relay, `/auth/mal/callback?code=abc&state=${state}`))
			.status,
	).toBe(200);
	expect(
		(await callback(relay, `/auth/mal/callback?code=abc&state=${state}`))
			.status,
	).toBe(400);
	expect(seen).toHaveLength(1);
	expect((await result(relay, session)).status).toBe(200);
});

test("concurrent callbacks exchange the code once", async () => {
	const { relay, seen } = fake(tokens);
	const { params } = await start(relay, "mal");
	const path = `/auth/mal/callback?code=abc&state=${params.get("state")}`;
	const statuses = (
		await Promise.all([callback(relay, path), callback(relay, path)])
	).map((r) => r.status);
	expect(statuses.sort()).toEqual([200, 400]);
	expect(seen).toHaveLength(1);
});

test("denied and failed exchanges report errors", async () => {
	const denied = fake(tokens);
	const first = await start(denied.relay, "mal");
	await callback(
		denied.relay,
		`/auth/mal/callback?error=access_denied&state=${first.params.get("state")}`,
	);
	const got = await result(denied.relay, first.session);
	expect([got.status, got.body]).toEqual([403, '{"error":"access_denied"}']);
	expect(denied.seen).toHaveLength(0);

	const { relay } = fake(() => "rejected");
	const { session, params } = await start(relay, "anilist");
	expect(
		(
			await callback(
				relay,
				`/auth/anilist/callback?code=c&state=${params.get("state")}`,
			)
		).status,
	).toBe(502);
	expect((await result(relay, session)).body).toBe(
		'{"error":"exchange_failed"}',
	);
	expect((await result(relay, session)).status).toBe(410);
});

test("other provider errors are failures", async () => {
	const { relay, seen } = fake(tokens);
	const { session, params } = await start(relay, "mal");
	const path = `/auth/mal/callback?error=server_error&code=c&state=${params.get("state")}`;
	expect((await callback(relay, path)).status).toBe(502);
	expect((await result(relay, session)).body).toBe(
		'{"error":"exchange_failed"}',
	);
	expect(seen).toHaveLength(0);
});

test("session cap answers busy", async () => {
	const { relay } = fake(tokens, {
		limits: { ttl: 600_000, maxSessions: 2 },
	});
	await start(relay, "mal");
	await start(relay, "anilist");
	const busy = await relay.handle("POST", "/auth/start", '{"service":"mal"}');
	expect([busy.status, busy.body]).toEqual([503, '{"error":"busy"}']);
});

test("expired sessions are gone", async () => {
	const { relay } = fake(tokens, { limits: { ttl: 0, maxSessions: 10 } });
	const { session, params } = await start(relay, "mal");
	expect((await result(relay, session)).status).toBe(410);
	expect(
		(
			await callback(
				relay,
				`/auth/mal/callback?code=abc&state=${params.get("state")}`,
			)
		).status,
	).toBe(400);
});

test("invalid requests are rejected", async () => {
	const { relay } = fake(tokens);
	expect(
		(await relay.handle("POST", "/auth/start", '{"service":"trakt"}'))
			.status,
	).toBe(400);
	expect((await relay.handle("POST", "/auth/start", "not json")).status).toBe(
		400,
	);
	expect((await relay.handle("GET", "/auth/result", "")).status).toBe(400);
	expect((await result(relay, "unknown")).status).toBe(410);
	expect(
		(await callback(relay, "/auth/trakt/callback?code=a&state=b")).status,
	).toBe(404);
	expect((await callback(relay, "/")).status).toBe(404);
	const malOnly = fake(tokens, {
		config: { ...config(), anilist: undefined },
	});
	expect(
		(
			await malOnly.relay.handle(
				"POST",
				"/auth/start",
				'{"service":"anilist"}',
			)
		).status,
	).toBe(503);
});

test("refresh uses the secret and maps errors", async () => {
	const { relay, seen } = fake(tokens);
	const got = await relay.handle(
		"POST",
		"/auth/refresh",
		'{"service":"mal","refresh_token":"r1"}',
	);
	expect(got.status).toBe(200);
	expect(JSON.parse(got.body)).toEqual(tokens());
	expect("form" in seen[0] && seen[0].form).toMatchObject({
		grant_type: "refresh_token",
		refresh_token: "r1",
		client_secret: "mal-secret",
	});

	expect(
		(
			await relay.handle(
				"POST",
				"/auth/refresh",
				'{"service":"anilist","refresh_token":"r"}',
			)
		).status,
	).toBe(400);
	expect(
		(
			await relay.handle(
				"POST",
				"/auth/refresh",
				'{"service":"mal","refresh_token":""}',
			)
		).status,
	).toBe(400);
	const body = '{"service":"mal","refresh_token":"r"}';
	expect(
		(
			await fake(() => "rejected").relay.handle(
				"POST",
				"/auth/refresh",
				body,
			)
		).status,
	).toBe(401);
	expect(
		(
			await fake(() => "unavailable").relay.handle(
				"POST",
				"/auth/refresh",
				body,
			)
		).status,
	).toBe(502);
});

test("only bad request and unauthorized reject tokens", () => {
	expect([400, 401, 403, 429, 500].map(exchangeError)).toEqual([
		"rejected",
		"rejected",
		"unavailable",
		"unavailable",
		"unavailable",
	]);
});

test("config validation matches the server's", () => {
	const ok = configFrom({
		PLAIVO_RELAY_PUBLIC_URL: " https://relay.test/ ",
		PLAIVO_MAL_CLIENT_ID: "id",
	});
	expect(ok).toEqual({
		publicUrl: "https://relay.test",
		mal: { id: "id", secret: undefined },
		anilist: undefined,
	});
	expect(() => configFrom({ PLAIVO_MAL_CLIENT_ID: "id" })).toThrow(
		"PLAIVO_RELAY_PUBLIC_URL",
	);
	expect(() =>
		configFrom({
			PLAIVO_RELAY_PUBLIC_URL: "https://x",
			PLAIVO_ANILIST_CLIENT_ID: "id",
		}),
	).toThrow("set both");
	expect(() => configFrom({ PLAIVO_RELAY_PUBLIC_URL: "https://x" })).toThrow(
		"at least one",
	);
});
