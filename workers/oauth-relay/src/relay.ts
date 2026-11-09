const TTL_MS = 600_000;
const MAX_SESSIONS = 65_536;
const MAX_PARAM = 2048;

const AUTHORIZE = {
	mal: "https://myanimelist.net/v1/oauth2/authorize",
} as const;
export const MAL_TOKEN = "https://myanimelist.net/v1/oauth2/token";

const EXPIRED =
	"This sign-in link has expired or was already used. Start again from Plaivo.";
const FAILED = "Sign-in failed. Close this tab and try again from Plaivo.";
const VERDICTS = {
	access_denied: [200, "Sign-in was cancelled. You can close this tab."],
	exchange_failed: [502, FAILED],
	signed_in: [200, "Signed in. You can close this tab and return to Plaivo."],
} as const;

const HTML = "text/html; charset=utf-8";
const JSON_TYPE = "application/json";
const USER_AGENT = "Plaivo-OAuth-Relay/1.0 (+https://plaivo.app)";

export type Service = "mal";
export type Client = { id: string; secret?: string };
export type Config = {
	publicUrl: string;
	mal: Client;
};
export type Tokens = {
	access_token: string;
	refresh_token?: string;
	expires_in?: number;
	token_type?: string;
};
export type TokenRequest = { url: string; form: Record<string, string> };
export type ExchangeError = "rejected" | "unavailable";
export type Exchange = (
	request: TokenRequest,
) => Promise<Tokens | ExchangeError>;
export type Reply = {
	status: number;
	contentType: string;
	body: string;
	location?: string;
};
export type Limits = { ttl: number; maxSessions: number };

export interface Sql {
	exec(
		query: string,
		...bindings: (string | number | null)[]
	): { toArray(): Record<string, unknown>[] };
}

type Outcome = Tokens | "access_denied" | "exchange_failed";

const verdict = (outcome: Outcome) =>
	VERDICTS[typeof outcome === "string" ? outcome : "signed_in"];

export const error = (status: number, code: string): Reply =>
	json(status, { error: code });

const json = (status: number, value: unknown): Reply => ({
	status,
	contentType: JSON_TYPE,
	body: JSON.stringify(value),
});

const page = (status: number, message: string): Reply => ({
	status,
	contentType: HTML,
	body:
		'<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width"><title>Plaivo</title>' +
		`<body style="font:18px system-ui,sans-serif;text-align:center;padding:15vh 1em"><h1>Plaivo</h1><p>${message}</p>`,
});

const parseService = (name: unknown): Service | undefined =>
	name === "mal" ? name : undefined;

function parseJson(body: string): Record<string, unknown> | undefined {
	try {
		const value: unknown = JSON.parse(body);
		return typeof value === "object" && value !== null
			? (value as Record<string, unknown>)
			: undefined;
	} catch {
		return undefined;
	}
}

function param(query: URLSearchParams, key: string): string | undefined {
	const value = query.get(key);
	return value && value.length <= MAX_PARAM ? value : undefined;
}

const randomHex = (bytes: number) =>
	Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (b) =>
		b.toString(16).padStart(2, "0"),
	).join("");

const malCredentials = (client: Client): Record<string, string> =>
	client.secret === undefined
		? { client_id: client.id }
		: { client_id: client.id, client_secret: client.secret };

export class Relay {
	constructor(
		private sql: Sql,
		private config: Config,
		private exchange: Exchange,
		private limits: Limits = { ttl: TTL_MS, maxSessions: MAX_SESSIONS },
	) {
		sql.exec(
			"CREATE TABLE IF NOT EXISTS sessions (id TEXT PRIMARY KEY, state TEXT UNIQUE, service TEXT NOT NULL, verifier TEXT NOT NULL, expires INTEGER NOT NULL, outcome TEXT)",
		);
	}

	async handle(method: string, url: string, body: string): Promise<Reply> {
		const at = url.indexOf("?");
		const path = at < 0 ? url : url.slice(0, at);
		const query = new URLSearchParams(at < 0 ? "" : url.slice(at + 1));
		if (method === "POST" && path === "/auth/start")
			return this.start(body);
		if (method === "GET" && path === "/auth/result")
			return this.result(query);
		if (method === "POST" && path === "/auth/refresh")
			return this.refresh(body);
		const [, name, callback] =
			/^\/auth\/([^/]*)(\/callback)?$/.exec(path) ?? [];
		const service = method === "GET" ? parseService(name) : undefined;
		if (!service) return error(404, "not_found");
		return callback
			? this.callback(service, query)
			: this.authorize(service, query);
	}

	// The app shows this short link (and its QR code); it redirects to the provider's long authorize URL.
	private authorize(service: Service, query: URLSearchParams): Reply {
		const state = param(query, "state");
		const [row] = state
			? this.sql
					.exec(
						"SELECT verifier FROM sessions WHERE state = ? AND service = ? AND outcome IS NULL AND expires > ?",
						state,
						service,
						Date.now(),
					)
					.toArray()
			: [];
		if (!state || !row) return page(400, EXPIRED);
		const target = new URLSearchParams({
			response_type: "code",
			client_id: this.config[service].id,
			redirect_uri: this.redirectUri(service),
			state,
			code_challenge: row.verifier as string,
			code_challenge_method: "plain",
		});
		return {
			status: 302,
			contentType: HTML,
			body: "",
			location: `${AUTHORIZE[service]}?${target}`,
		};
	}

	private redirectUri(service: Service) {
		return `${this.config.publicUrl}/auth/${service}/callback`;
	}

	private start(body: string): Reply {
		const service = parseService(parseJson(body)?.service);
		if (!service) return error(400, "invalid_service");
		const [id, state, verifier] = [
			randomHex(32),
			randomHex(32),
			randomHex(48),
		];
		const now = Date.now();
		this.sql.exec("DELETE FROM sessions WHERE expires <= ?", now);
		const [{ n }] = this.sql
			.exec("SELECT COUNT(*) AS n FROM sessions")
			.toArray();
		if ((n as number) >= this.limits.maxSessions) return error(503, "busy");
		this.sql.exec(
			"INSERT INTO sessions (id, state, service, verifier, expires) VALUES (?, ?, ?, ?, ?)",
			id,
			state,
			service,
			verifier,
			now + this.limits.ttl,
		);
		return json(200, {
			session: id,
			authorize_url: `${this.config.publicUrl}/auth/${service}?state=${state}`,
			expires_in: this.limits.ttl / 1000,
		});
	}

	private async callback(
		service: Service,
		query: URLSearchParams,
	): Promise<Reply> {
		const state = param(query, "state");
		const claimed = state && this.claim(service, state);
		if (!claimed) return page(400, EXPIRED);

		const [providerError, code] = [
			param(query, "error"),
			param(query, "code"),
		];
		let outcome: Outcome = "exchange_failed";
		if (providerError === "access_denied") outcome = "access_denied";
		else if (!providerError && code) {
			const got = await this.exchange(
				this.codeRequest(code, claimed.verifier),
			);
			if (typeof got !== "string") outcome = got;
		}
		this.settle(claimed.id, outcome);
		const [status, message] = verdict(outcome);
		return page(status, message);
	}

	// Claimed synchronously so a second callback can't reuse the state while the exchange is awaited.
	private claim(service: Service, state: string) {
		const [row] = this.sql
			.exec(
				"UPDATE sessions SET state = NULL WHERE state = ? AND service = ? AND outcome IS NULL AND expires > ? RETURNING id, verifier",
				state,
				service,
				Date.now(),
			)
			.toArray();
		return (
			row && { id: row.id as string, verifier: row.verifier as string }
		);
	}

	private settle(id: string, outcome: Outcome) {
		this.sql.exec(
			"UPDATE sessions SET outcome = ?, expires = ? WHERE id = ?",
			JSON.stringify(
				typeof outcome === "string"
					? { error: outcome }
					: { tokens: outcome },
			),
			Date.now() + this.limits.ttl,
			id,
		);
	}

	private codeRequest(code: string, verifier: string): TokenRequest {
		return {
			url: MAL_TOKEN,
			form: {
				...malCredentials(this.config.mal),
				grant_type: "authorization_code",
				code,
				redirect_uri: this.redirectUri("mal"),
				code_verifier: verifier,
			},
		};
	}

	private result(query: URLSearchParams): Reply {
		const id = param(query, "session");
		if (!id) return error(400, "invalid_session");
		const [row] = this.sql
			.exec("SELECT expires, outcome FROM sessions WHERE id = ?", id)
			.toArray();
		const live = row !== undefined && (row.expires as number) > Date.now();
		if (live && row.outcome === null)
			return { status: 204, contentType: JSON_TYPE, body: "" };
		if (row) this.sql.exec("DELETE FROM sessions WHERE id = ?", id);
		if (!live) return error(410, "expired");
		const outcome = JSON.parse(row.outcome as string) as
			| { tokens: Tokens }
			| { error: string };
		return "tokens" in outcome
			? json(200, outcome.tokens)
			: error(403, outcome.error);
	}

	private async refresh(body: string): Promise<Reply> {
		const request = parseJson(body);
		const token = request?.refresh_token;
		if (
			typeof request?.service !== "string" ||
			typeof token !== "string" ||
			!token ||
			token.length > MAX_PARAM
		) {
			return error(400, "invalid_request");
		}
		const service = parseService(request.service);
		if (!service) return error(400, "invalid_service");
		const client = this.config.mal;
		const got = await this.exchange({
			url: MAL_TOKEN,
			form: {
				...malCredentials(client),
				grant_type: "refresh_token",
				refresh_token: token,
			},
		});
		if (got === "rejected") return error(401, "refresh_rejected");
		if (got === "unavailable") return error(502, "provider_unavailable");
		return json(200, got);
	}
}

export function configFrom(env: Record<string, unknown>): Config {
	const read = (name: string) => {
		const value = env[name];
		return typeof value === "string" && value.trim()
			? value.trim()
			: undefined;
	};
	const publicUrl = read("PLAIVO_RELAY_PUBLIC_URL");
	if (!publicUrl || !/^https?:\/\//.test(publicUrl)) {
		throw new Error(
			"PLAIVO_RELAY_PUBLIC_URL must be set to the relay's public http(s) URL",
		);
	}
	const mal = read("PLAIVO_MAL_CLIENT_ID");
	if (!mal) throw new Error("PLAIVO_MAL_CLIENT_ID must be set");
	return {
		publicUrl: publicUrl.replace(/\/+$/, ""),
		mal: { id: mal, secret: read("PLAIVO_MAL_CLIENT_SECRET") },
	};
}

export const exchangeError = (status: number): ExchangeError =>
	status === 400 || status === 401 ? "rejected" : "unavailable";

function tokensFrom(value: unknown): Tokens | undefined {
	const v = (
		typeof value === "object" && value !== null ? value : {}
	) as Record<string, unknown>;
	if (typeof v.access_token !== "string") return undefined;
	const tokens: Tokens = { access_token: v.access_token };
	if (typeof v.refresh_token === "string")
		tokens.refresh_token = v.refresh_token;
	if (Number.isSafeInteger(v.expires_in) && (v.expires_in as number) >= 0)
		tokens.expires_in = v.expires_in as number;
	if (typeof v.token_type === "string") tokens.token_type = v.token_type;
	return tokens;
}

export async function fetchTokens(
	request: TokenRequest,
): Promise<Tokens | ExchangeError> {
	try {
		const response = await fetch(request.url, {
			method: "POST",
			headers: { Accept: JSON_TYPE, "User-Agent": USER_AGENT },
			body: new URLSearchParams(request.form),
			signal: AbortSignal.timeout(15_000),
		});
		if (!response.ok) {
			console.error(
				`token request to ${request.url} answered ${response.status}: ${(await response.text()).slice(0, 300)}`,
			);
			return exchangeError(response.status);
		}
		const tokens = tokensFrom(await response.json());
		if (tokens) return tokens;
		console.error(
			`token request to ${request.url} returned no access_token`,
		);
	} catch (e) {
		console.error(`token request to ${request.url} failed: ${e}`);
	}
	return "unavailable";
}
