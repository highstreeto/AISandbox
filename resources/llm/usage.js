// Token usage for the llm gateway (njs body filter).
//
// The response is passed on unchanged. On the way, the model and the token counts are read from it into the
// $usage_* variables, which the "usage" log format writes after the request:
//   Ollama      {"model", "prompt_eval_count", "eval_count"}            (NDJSON when streamed)
//   OpenAI      {"model", "usage": {"prompt_tokens", "completion_tokens"}}
//               streamed: only if the client asks for stream_options.include_usage, else the counts stay null
//   Anthropic   message_start {"message": {"model", "usage": {"input_tokens"}}} and
//               message_delta {"usage": {"output_tokens"}}              (SSE when streamed)
// Streamed answers (NDJSON / SSE) are read line by line, with a partial last line kept for the next chunk;
// other answers as one document (not above MAX_BODY: the counts stay null).

const MAX_BODY = 4 * 1024 * 1024;
const GAVE_UP = '\u0000';   // $usage_buf after the body grew past MAX_BODY

function filter(r, data, flags) {
    r.sendBuffer(data, flags);
    try {
        scan(r, data.toString(), flags.last);
    } catch (e) {
        r.warn('usage: ' + e);
    }
}

function scan(r, text, last) {
    const v = r.variables;
    const type = String(r.headersOut['Content-Type'] || '');
    const streamed = type.indexOf('event-stream') >= 0 || type.indexOf('ndjson') >= 0;
    v.usage_stream = streamed ? 'true' : 'false';
    const buf = v.usage_buf + text;

    if (streamed) {
        // whole lines now; the partial last line waits for the next chunk (at the end it is a line too)
        const lines = buf.split('\n');
        v.usage_buf = last ? '' : lines.pop();
        for (const line of lines) {
            fromLine(r, line);
        }
    } else if (buf.length > MAX_BODY || v.usage_buf === GAVE_UP) {
        v.usage_buf = last ? '' : GAVE_UP;
    } else if (last) {
        v.usage_buf = '';
        fromDocument(r, buf);
    } else {
        v.usage_buf = buf;
    }
}

function fromLine(r, line) {
    let s = line.trim();
    if (s.startsWith('data:')) {
        s = s.slice(5).trim();
    }
    if (s.charAt(0) === '{') {
        fromDocument(r, s);
    }
}

function fromDocument(r, text) {
    let o;
    try {
        o = JSON.parse(text);
    } catch (e) {
        return;
    }
    const v = r.variables;
    const model = o.model || (o.message && o.message.model);
    if (model) {
        v.usage_model = String(model);
    }
    set(v, 'usage_in', o.prompt_eval_count);
    set(v, 'usage_out', o.eval_count);
    const u = o.usage || (o.message && o.message.usage);
    if (u) {
        set(v, 'usage_in', u.prompt_tokens);
        set(v, 'usage_out', u.completion_tokens);
        set(v, 'usage_in', u.input_tokens);
        set(v, 'usage_out', u.output_tokens);
    }
}

function set(v, name, n) {
    if (typeof n === 'number' && isFinite(n)) {
        v[name] = String(Math.trunc(n));
    }
}

export default { filter };
