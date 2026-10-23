/**
 * Herdcats Landing Page Server
 * Lightweight, zero-dependency Node.js HTTP server.
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const DEFAULT_PORT = 3000;
const OUT_DIR = path.join(__dirname, 'out');

function getPublicDir() {
    if (process.env.PUBLIC_DIR) {
        return path.resolve(process.env.PUBLIC_DIR);
    }
    return fs.existsSync(OUT_DIR) ? OUT_DIR : path.join(__dirname, 'public');
}

// Parse --port argument or PORT env
function getPort() {
    const portArgIdx = process.argv.indexOf('--port');
    if (portArgIdx !== -1 && process.argv[portArgIdx + 1]) {
        const p = parseInt(process.argv[portArgIdx + 1], 10);
        if (!isNaN(p)) return p;
    }
    const envPort = parseInt(process.env.PORT, 10);
    if (!isNaN(envPort)) return envPort;
    return DEFAULT_PORT;
}

const MIME_TYPES = {
    '.html': 'text/html; charset=utf-8',
    '.css': 'text/css; charset=utf-8',
    '.js': 'application/javascript; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.txt': 'text/plain; charset=utf-8',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.jpg': 'image/jpeg',
    '.jpeg': 'image/jpeg',
    '.ico': 'image/x-icon',
    '.webp': 'image/webp',
    '.woff2': 'font/woff2',
    '.woff': 'font/woff',
    '.ttf': 'font/ttf'
};

function handleRequest(req, res) {
    // Basic CORS & security headers
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.setHeader('X-Frame-Options', 'SAMEORIGIN');

    let pathname;
    try {
        // Routing is based only on the request target. Never interpret an
        // untrusted Host header as the URL base.
        const url = new URL(req.url, 'http://localhost');
        pathname = decodeURIComponent(url.pathname);
        if (pathname.includes('\0')) {
            throw new URIError('NUL byte in request path');
        }
    } catch (_error) {
        res.writeHead(400, { 'Content-Type': 'text/plain' });
        res.end('400 Bad Request');
        return;
    }

    // API status health check
    if (pathname === '/api/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ status: 'ok', app: 'Herdcats Landing', uptime: process.uptime() }));
        return;
    }

    if (pathname === '/') {
        pathname = '/index.html';
    }

    const publicDir = req.publicDir || getPublicDir();

    // Prevent directory traversal
    const safePath = path.normalize(pathname).replace(/^(\.\.[\/\\])+/, '');
    let filePath = path.join(publicDir, safePath);

    // Verify file is within publicDir using a strict path-boundary check
    // (a bare string prefix would also accept sibling dirs like `<publicDir>-secret`).
    const rel = path.relative(publicDir, filePath);
    if (
        rel === '..' ||
        rel.startsWith('..' + path.sep) ||
        path.isAbsolute(rel) ||
        !filePath.startsWith(publicDir)
    ) {
        res.writeHead(403, { 'Content-Type': 'text/plain' });
        res.end('403 Forbidden');
        return;
    }

    function serveFile(targetPath) {
        const ext = path.extname(targetPath).toLowerCase();
        const contentType = MIME_TYPES[ext] || 'application/octet-stream';

        fs.readFile(targetPath, (readErr, content) => {
            if (readErr) {
                res.writeHead(500, { 'Content-Type': 'text/plain' });
                res.end('500 Internal Server Error');
                return;
            }

            const acceptEncoding = req.headers['accept-encoding'] || '';
            const shouldCompress = /text|javascript|json|xml/.test(contentType) && content.length > 1024;

            if (shouldCompress && acceptEncoding.includes('gzip')) {
                zlib.gzip(content, (gzipErr, gzipped) => {
                    if (gzipErr) {
                        res.writeHead(200, { 'Content-Type': contentType });
                        res.end(content);
                        return;
                    }
                    res.writeHead(200, {
                        'Content-Type': contentType,
                        'Content-Encoding': 'gzip',
                        'Content-Length': gzipped.length,
                        'Cache-Control': 'public, max-age=3600'
                    });
                    res.end(gzipped);
                });
            } else {
                res.writeHead(200, {
                    'Content-Type': contentType,
                    'Content-Length': content.length,
                    'Cache-Control': 'public, max-age=3600'
                });
                res.end(content);
            }
        });
    }

    fs.stat(filePath, (err, stats) => {
        if (!err && stats.isFile()) {
            serveFile(filePath);
            return;
        }

        if (!err && stats.isDirectory()) {
            const indexFilePath = path.join(filePath, 'index.html');
            fs.stat(indexFilePath, (indexErr, indexStats) => {
                if (!indexErr && indexStats.isFile()) {
                    serveFile(indexFilePath);
                    return;
                }
                // Fallback to index.html for SPA if not found
                serveFile(path.join(publicDir, 'index.html'));
            });
            return;
        }

        // Check if filePath + '.html' exists
        const htmlFilePath = filePath + '.html';
        fs.stat(htmlFilePath, (htmlErr, htmlStats) => {
            if (!htmlErr && htmlStats.isFile()) {
                serveFile(htmlFilePath);
                return;
            }
            // Fallback to index.html for SPA if not found
            serveFile(path.join(publicDir, 'index.html'));
        });
    });
}

function createServer() {
    return http.createServer(handleRequest);
}

function start() {
    const server = createServer();
    const PORT = getPort();

    server.listen(PORT, '0.0.0.0', () => {
        console.log(`\n🐾 Herdcats Landing Server running!`);
        console.log(`   ➜ Local:   http://localhost:${PORT}`);
        console.log(`   ➜ Network: http://0.0.0.0:${PORT}\n`);
    });

    // Handle graceful termination
    process.on('SIGINT', () => {
        server.close(() => process.exit(0));
    });
    process.on('SIGTERM', () => {
        server.close(() => process.exit(0));
    });
    return server;
}

if (require.main === module) {
    start();
}

module.exports = { createServer, handleRequest, start, getPublicDir };
