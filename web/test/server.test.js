const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test, after } = require('node:test');

// Create an isolated export fixture so tests pass in clean checkouts without prebuilding
const fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), 'herdcats-server-test-'));
process.env.PUBLIC_DIR = fixtureDir;

fs.writeFileSync(path.join(fixtureDir, 'index.html'), '<!DOCTYPE html><html><body><h1>Herdcats</h1></body></html>');
fs.writeFileSync(path.join(fixtureDir, 'third-party-notices.txt'), 'License notices — copyright holders retain their rights.');

fs.mkdirSync(path.join(fixtureDir, 'about'), { recursive: true });
fs.writeFileSync(path.join(fixtureDir, 'about', 'index.html'), '<!DOCTYPE html><html><body><h1>About Herdcats</h1></body></html>');

fs.mkdirSync(path.join(fixtureDir, 'privacy'), { recursive: true });
fs.writeFileSync(path.join(fixtureDir, 'privacy', 'index.html'), '<!DOCTYPE html><html><body><h1>Privacy Policy</h1></body></html>');

fs.mkdirSync(path.join(fixtureDir, 'terms'), { recursive: true });
fs.writeFileSync(path.join(fixtureDir, 'terms', 'index.html'), '<!DOCTYPE html><html><body><h1>Terms</h1></body></html>');

after(() => {
    fs.rmSync(fixtureDir, { recursive: true, force: true });
    delete process.env.PUBLIC_DIR;
});

const { handleRequest } = require('../server');

function request(path, headers = {}) {
    const response = {
        headers: {},
        setHeader(name, value) { this.headers[name] = value; },
        writeHead(statusCode, headers = {}) {
            this.statusCode = statusCode;
            Object.assign(this.headers, headers);
        },
        end(body = '') { this.body = body; },
    };
    handleRequest({ url: path, headers }, response);
    return response;
}

function requestAsync(path, headers = {}) {
    return new Promise((resolve) => {
        const response = {
            headers: {},
            setHeader(name, value) { this.headers[name] = value; },
            writeHead(statusCode, headers = {}) {
                this.statusCode = statusCode;
                Object.assign(this.headers, headers);
            },
            end(body = '') {
                this.body = typeof body === 'string' ? body : body.toString('utf8');
                resolve(this);
            },
        };
        handleRequest({ url: path, headers }, response);
    });
}

test('malformed targets return 400 and later requests still succeed', () => {
    assert.equal(request('/%').statusCode, 400);
    assert.equal(request('/%00').statusCode, 400);
    assert.equal(request('/api/health', { host: '[' }).statusCode, 200);
    assert.equal(request('/api/health').statusCode, 200);
});

test('exported route directories resolve to their index.html', async () => {
    const privacy = await requestAsync('/privacy');
    assert.equal(privacy.statusCode, 200);
    assert.match(privacy.body, /Privacy Policy/);

    const privacyTrailing = await requestAsync('/privacy/');
    assert.equal(privacyTrailing.statusCode, 200);
    assert.match(privacyTrailing.body, /Privacy Policy/);

    const about = await requestAsync('/about');
    assert.equal(about.statusCode, 200);
    assert.match(about.body, /About Herdcats/);

    const terms = await requestAsync('/terms');
    assert.equal(terms.statusCode, 200);
    assert.match(terms.body, /Terms/);

    const root = await requestAsync('/');
    assert.equal(root.statusCode, 200);
    assert.match(root.body, /Herdcats/);
});

test('license notices are served as readable UTF-8 text', async () => {
    const response = await requestAsync('/third-party-notices.txt');
    assert.equal(response.statusCode, 200);
    assert.equal(response.headers['Content-Type'], 'text/plain; charset=utf-8');
    assert.equal(response.body, 'License notices — copyright holders retain their rights.');
});

test('traversal attempts never escape the public directory or reach sibling prefixes', async () => {
    const siblingDir = `${fixtureDir}-secret`;
    fs.mkdirSync(siblingDir, { recursive: true });
    fs.writeFileSync(path.join(siblingDir, 'secret.txt'), 'top secret');
    try {
        const siblingName = path.basename(siblingDir);
        const attempts = [
            `/../${siblingName}/secret.txt`,
            `/..%2f${siblingName}%2fsecret.txt`,
            `/%2e%2e/${siblingName}/secret.txt`,
            `/about/../../${siblingName}/secret.txt`,
        ];
        for (const attempt of attempts) {
            const response = await requestAsync(attempt);
            assert.doesNotMatch(response.body, /top secret/, attempt);
        }
    } finally {
        fs.rmSync(siblingDir, { recursive: true, force: true });
    }
});
