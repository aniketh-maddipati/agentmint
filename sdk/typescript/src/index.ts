export type Actor = {
  subject: string;
  agentId: string;
  delegatedBy?: string | null;
  issuer: string;
};

export type ResourceRef = {
  type: string;
  id: string;
};

export type ActionContext = {
  supportTicketId?: string;
  reason?: string;
};

export type ProposeInput = {
  tenantId: string;
  actor: Actor;
  provider: string;
  operation: string;
  resource: ResourceRef;
  arguments: Record<string, unknown>;
  context?: ActionContext;
  ttlSeconds?: number;
};

export type Action = {
  id: string;
  status: string;
  tenantId: string;
  intentHash: string;
  provider: string;
  operation: string;
  resource: ResourceRef;
  arguments: Record<string, unknown>;
  context: ActionContext;
  actor: {
    subject: string;
    agent_id: string;
    delegated_by?: string | null;
    issuer: string;
  };
  expiresAt: string;
  policy?: {
    effect: string;
    policy_version: string;
    reason: string;
  };
  providerResourceId?: string | null;
  reconciliationRequired: boolean;
};

export type MintOptions = {
  baseUrl: string;
  token: string;
  fetch?: typeof fetch;
};

export class MintError extends Error {
  readonly status: number;
  readonly code: string;
  readonly retryable: boolean;

  constructor(status: number, code: string, message: string, retryable: boolean) {
    super(message);
    this.status = status;
    this.code = code;
    this.retryable = retryable;
  }
}

export class Mint {
  readonly actions: {
    propose: (input: ProposeInput) => Promise<Action>;
    get: (id: string) => Promise<Action>;
    approve: (id: string, intentHash: string) => Promise<Action>;
    deny: (id: string, intentHash: string) => Promise<Action>;
    execute: (id: string, arguments_?: Record<string, unknown>) => Promise<Action>;
    reconcile: (id: string) => Promise<Action>;
    receipt: (id: string) => Promise<unknown>;
  };
  private readonly options: MintOptions;

  constructor(options: MintOptions) {
    this.options = options;
    this.actions = {
      propose: (input) => this.request("POST", "/v1/actions", input),
      get: (id) => this.request("GET", `/v1/actions/${id}`),
      approve: (id, intentHash) =>
        this.request("POST", `/v1/actions/${id}/approve`, { intentHash }),
      deny: (id, intentHash) =>
        this.request("POST", `/v1/actions/${id}/deny`, { intentHash }),
      execute: (id, arguments_) =>
        this.request("POST", `/v1/actions/${id}/execute`, arguments_ ? { arguments: arguments_ } : {}),
      reconcile: (id) => this.request("POST", `/v1/actions/${id}/reconcile`, {}),
      receipt: (id) => this.request("GET", `/v1/actions/${id}/receipt`),
    };
  }

  keys(): Promise<{ keys: Array<Record<string, string>> }> {
    return this.request("GET", "/v1/keys");
  }

  private async request<T = Action>(method: string, path: string, body?: unknown): Promise<T> {
    const fetchImpl = this.options.fetch ?? fetch;
    const response = await fetchImpl(`${this.options.baseUrl}${path}`, {
      method,
      headers: {
        authorization: this.options.token.startsWith("Bearer ")
          ? this.options.token
          : `Bearer ${this.options.token}`,
        "content-type": "application/json",
        accept: "application/json",
      },
      body: method === "GET" || body === undefined ? undefined : JSON.stringify(body),
    });
    const payload = await response.json();
    if (!response.ok) {
      const error = payload?.error ?? {};
      throw new MintError(
        response.status,
        error.code ?? "unknown",
        error.message ?? response.statusText,
        Boolean(error.retryable),
      );
    }
    return payload as T;
  }
}

export function encodeDevToken(identity: {
  tenantId: string;
  subject: string;
  agentId: string;
  issuer: string;
  delegatedBy?: string | null;
}): string {
  const json = JSON.stringify({
    tenant_id: identity.tenantId,
    subject: identity.subject,
    agent_id: identity.agentId,
    issuer: identity.issuer,
    delegated_by: identity.delegatedBy ?? null,
  });
  return `dev.${Buffer.from(json).toString("base64url")}`;
}
