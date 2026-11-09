import { DurableObject } from "cloudflare:workers";
import { configFrom, error, fetchTokens, Relay, type Reply } from "./relay";

const MAX_BODY = 4096;

type Env = {
	SESSIONS: DurableObjectNamespace<Sessions>;
	LIMITER: RateLimit;
	PLAIVO_RELAY_PUBLIC_URL?: string;
	PLAIVO_MAL_CLIENT_ID?: string;
	PLAIVO_MAL_CLIENT_SECRET?: string;
};

export class Sessions extends DurableObject<Env> {
	private relay: Relay;

	constructor(ctx: DurableObjectState, env: Env) {
		super(ctx, env);
		this.relay = new Relay(ctx.storage.sql, configFrom(env), fetchTokens);
	}

	handle(method: string, url: string, body: string): Promise<Reply> {
		return this.relay.handle(method, url, body);
	}
}

async function route(request: Request, env: Env): Promise<Reply> {
	const { success } = await env.LIMITER.limit({
		key: request.headers.get("CF-Connecting-IP") ?? "",
	});
	if (!success) return error(429, "rate_limited");
	if (Number(request.headers.get("Content-Length")) > MAX_BODY)
		return error(413, "body_too_large");
	const body = await request.arrayBuffer();
	if (body.byteLength > MAX_BODY) return error(413, "body_too_large");
	const url = new URL(request.url);
	const text = new TextDecoder().decode(body);
	return env.SESSIONS.getByName("relay").handle(
		request.method,
		url.pathname + url.search,
		text,
	);
}

export default {
	async fetch(request, env) {
		const reply = await route(request, env);
		return new Response(reply.status === 204 ? null : reply.body, {
			status: reply.status,
			headers: {
				"Content-Type": reply.contentType,
				"Cache-Control": "no-store",
				"Referrer-Policy": "no-referrer",
				...(reply.location && { Location: reply.location }),
			},
		});
	},
} satisfies ExportedHandler<Env>;
