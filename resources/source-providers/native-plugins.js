"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const node_child_process_1 = require("node:child_process");
const node_util_1 = require("node:util");
const node_os_1 = require("node:os");
const promises_1 = require("node:fs/promises");
const node_path_1 = require("node:path");
const execute = (0, node_util_1.promisify)(node_child_process_1.execFile);
async function run(program, args) {
    return (await execute(program, args, {
        timeout: 90_000, maxBuffer: 1_048_576,
        env: { ...process.env, SM_INTERNAL_RUN: '1' },
    })).stdout;
}
function sameSource(left, right) {
    return left.replace(/\.git\/?$/, '').replace(/\/$/, '') === right.replace(/\.git\/?$/, '').replace(/\/$/, '');
}
async function synchronizeCodex(source, selector) {
    if (!/^[\w.-]+@[\w.-]+$/.test(selector))
        throw new Error('invalid native plugin selector');
    if (process.env.SM_INSTALL_HOME && process.env.CODEX_HOME && process.env.CODEX_HOME !== (0, node_path_1.join)((0, node_os_1.homedir)(), '.codex')) {
        throw new Error('native installer configuration root differs from declared home');
    }
    const marketplace = selector.slice(selector.indexOf('@') + 1);
    // Curated marketplaces are distributed and refreshed by Codex itself.
    if (marketplace !== 'openai-api-curated') {
        const listed = JSON.parse(await run('codex', ['plugin', 'marketplace', 'list', '--json']));
        if (!listed || typeof listed !== 'object' || !('marketplaces' in listed) || !Array.isArray(listed.marketplaces)) {
            throw new Error('invalid native marketplace list');
        }
        const existing = listed.marketplaces.find((entry) => entry && typeof entry === 'object' && 'name' in entry && entry.name === marketplace);
        if (existing === undefined) {
            await run('codex', ['plugin', 'marketplace', 'add', source, '--json']);
        }
        else {
            const metadata = existing && typeof existing === 'object' && 'marketplaceSource' in existing ? existing.marketplaceSource : undefined;
            if (!metadata || typeof metadata !== 'object' || !('sourceType' in metadata) || !('source' in metadata) || typeof metadata.source !== 'string') {
                throw new Error('native marketplace source unavailable');
            }
            if (metadata.sourceType === 'git') {
                if (!sameSource(metadata.source, source))
                    throw new Error('native marketplace source differs from declaration');
                await run('codex', ['plugin', 'marketplace', 'upgrade', marketplace, '--json']);
            }
            else if (metadata.sourceType === 'local') {
                const root = await (0, promises_1.realpath)(metadata.source);
                const inside = (0, node_path_1.relative)(await (0, promises_1.realpath)((0, node_os_1.homedir)()), root);
                if (!inside || inside.startsWith('..') || (0, node_path_1.isAbsolute)(inside))
                    throw new Error('native marketplace outside home');
                if (!sameSource((await run('git', ['-C', root, 'remote', 'get-url', 'origin'])).trim(), source)) {
                    throw new Error('native marketplace Git source differs from declaration');
                }
                await run('git', ['-C', root, 'pull', '--ff-only']);
            }
            else
                throw new Error('native marketplace source unsupported');
        }
    }
    await run('codex', ['plugin', 'add', selector, '--json']);
}
async function synchronize() {
    const [agent, source, selector, enabledText, ...extra] = process.argv.slice(2);
    if (!['kimi', 'codex'].includes(agent ?? '') || !source || !selector || extra.length || !['true', 'false'].includes(enabledText ?? '')) {
        throw new Error('invalid native plugin arguments');
    }
    if (process.env.SM_INSTALL_HOME && process.env.SM_INSTALL_HOME !== (0, node_os_1.homedir)()) {
        throw new Error('native installer home differs from runtime home');
    }
    if (agent === 'codex')
        return synchronizeCodex(source, selector);
    if (process.env.SM_INSTALL_HOME && process.env.KIMI_CODE_HOME && process.env.KIMI_CODE_HOME !== (0, node_path_1.join)((0, node_os_1.homedir)(), '.kimi-code')) {
        throw new Error('native installer configuration root differs from declared home');
    }
    const enabled = enabledText === 'true';
    const child = (0, node_child_process_1.spawn)('kimi', ['web', '--host', '127.0.0.1', '--port', '0', '--no-open'], {
        stdio: ['ignore', 'pipe', 'pipe'],
        env: { ...process.env, KIMI_CODE_NO_AUTO_UPDATE: '1', SM_INTERNAL_RUN: '1' },
    });
    const exited = new Promise((resolve) => child.once('close', () => resolve()));
    const deadline = setTimeout(() => child.kill('SIGKILL'), 110_000);
    let output = '';
    let ready = false;
    try {
        const origin = await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('native server readiness failed')), 20_000);
            const finish = (error, address) => {
                clearTimeout(timeout);
                if (error)
                    reject(error);
                else if (address)
                    resolve(address);
            };
            const inspect = (chunk) => {
                if (ready)
                    return;
                output += chunk.toString('utf8');
                if (output.length > 131_072) {
                    ready = true;
                    output = '';
                    return finish(new Error('native server output limit exceeded'));
                }
                const address = output.replace(/\x1b\[[0-9;]*m/g, '').match(/http:\/\/127\.0\.0\.1:\d+(?=[/\s])/);
                if (address) {
                    ready = true;
                    output = '';
                    finish(undefined, address[0]);
                }
            };
            child.stdout.on('data', inspect);
            child.stderr.on('data', inspect);
            child.once('error', () => finish(new Error('native server could not start')));
            child.once('exit', () => finish(new Error('native server exited before readiness')));
        });
        const token = (await (0, promises_1.readFile)((0, node_path_1.join)(process.env.KIMI_CODE_HOME ?? (0, node_path_1.join)((0, node_os_1.homedir)(), '.kimi-code'), 'server.token'), 'utf8')).trim();
        if (!token || /[\r\n]/.test(token))
            throw new Error('native server token unavailable');
        const request = async (path, body) => {
            const response = await fetch(`${origin}/api/v1${path}`, {
                method: body === undefined ? 'GET' : 'POST',
                headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
                body: body === undefined ? null : JSON.stringify(body),
                signal: AbortSignal.timeout(90_000),
            });
            if (!response.ok || !response.body)
                throw new Error('native plugin API request failed');
            const chunks = [];
            let size = 0;
            for await (const chunk of response.body) {
                size += chunk.length;
                if (size > 1_048_576)
                    throw new Error('native plugin API response limit exceeded');
                chunks.push(chunk);
            }
            const result = JSON.parse(Buffer.concat(chunks).toString('utf8'));
            if (!result || typeof result !== 'object' || !('code' in result) || result.code !== 0 || !('data' in result)) {
                throw new Error('native plugin API operation failed');
            }
            return result.data;
        };
        try {
            const installed = await request('/plugins', { source });
            if (!installed || typeof installed !== 'object' || !('id' in installed) || installed.id !== selector) {
                throw new Error('native installer returned another plugin');
            }
        }
        finally {
            await request(`/plugins/${encodeURIComponent(selector)}:${enabled ? 'enable' : 'disable'}`, {});
        }
        const listed = await request('/plugins');
        const plugins = listed && typeof listed === 'object' && 'plugins' in listed ? listed.plugins : undefined;
        if (!Array.isArray(plugins) || !plugins.some((plugin) => plugin && typeof plugin === 'object' && 'id' in plugin && plugin.id === selector && 'enabled' in plugin && plugin.enabled === enabled && 'state' in plugin && plugin.state === 'ok')) {
            throw new Error('native plugin verification failed');
        }
    }
    finally {
        clearTimeout(deadline);
        child.kill('SIGTERM');
        const force = setTimeout(() => child.kill('SIGKILL'), 2_000);
        await exited;
        clearTimeout(force);
    }
}
synchronize().catch(() => {
    // Native API errors and startup banners may contain local tokens or configuration.
    process.stderr.write('Native plugin synchronization failed.\n');
    process.exitCode = 1;
});
