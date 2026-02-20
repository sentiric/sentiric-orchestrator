import { WebSocketStream } from './websocket.js';

const state = {
    services: [],
    nodes: [],
    logSocket: null,
};

const ui = {
    grid: document.getElementById('services-grid'),
    logModal: document.getElementById('log-modal'),
    logOutput: document.getElementById('log-output'),
    logTitle: document.getElementById('log-modal-title'),
    
    render() {
        if (!state.services.length && this.grid.innerHTML.includes('Waiting')) {
             this.grid.innerHTML = `<div style="color: #444; text-align: center; grid-column: 1/-1; padding-top: 50px;">No services detected. Is Docker running?</div>`;
             return;
        }

        const sorted = [...state.services].sort((a, b) => {
            if (a.has_gpu && !b.has_gpu) return -1;
            if (!a.has_gpu && b.has_gpu) return 1;
            return a.name.localeCompare(b.name);
        });

        this.grid.innerHTML = sorted.map(svc => this.createCard(svc)).join('');
    },

    createCard(svc) {
        const isRunning = svc.status.toLowerCase().includes('up');
        const statusColor = isRunning ? 'var(--color-accent)' : 'var(--color-warn)';
        
        return `
            <div class="card">
                <div class="card-content">
                    <div class="card-header">
                        <div>
                            <span class="card-title">${svc.name}</span>
                            ${svc.has_gpu ? '<span class="gpu-badge">GPU</span>' : ''}
                        </div>
                        <span class="status-text" style="color: ${statusColor};">
                            ${svc.status}
                        </span>
                    </div>
                    <div style="font-family: var(--font-mono); font-size:11px; color: var(--text-muted); margin-top: -10px;">
                        <p>Image: ${svc.image}</p>
                        <p>ID: ${svc.short_id}</p>
                    </div>
                </div>

                <div class="btn-group">
                    <button class="btn-toggle" onclick="window.nexus.toggleAP('${svc.name}', ${!svc.auto_pilot})">
                        ${svc.auto_pilot ? '🟢 AUTO-PILOT ON' : '⚪ AUTO-PILOT OFF'}
                    </button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'start')" ${isRunning ? 'disabled' : ''}>START</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'stop')" ${!isRunning ? 'disabled' : ''}>STOP</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'restart')" ${!isRunning ? 'disabled' : ''}>RESTART</button>
                    <button class="btn-accent" onclick="window.nexus.showLogs('${svc.short_id}', '${svc.name}')">LOGS</button>
                    <button class="btn-accent" onclick="window.nexus.updateSvc('${svc.name}')">RECREATE</button>
                </div>
            </div>
        `;
    },
    
    showLogModal(id, name) {
        this.logTitle.innerText = `Logs for: ${name}`;
        this.logOutput.innerHTML = ''; // Clear previous logs
        this.logModal.style.display = 'flex';
        
        // Connect to log websocket
        if (state.logSocket) state.logSocket.close();
        
        const decoder = new TextDecoder();
        state.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        state.logSocket.onopen = () => this.logOutput.innerHTML += '<span style="color: var(--color-accent)">[Connected to log stream...]</span>\n';
        state.logSocket.onmessage = (event) => {
            // ANSI color codes need a library, for now, just append text
            const text = decoder.decode(event.data);
            this.logOutput.innerHTML += text;
            this.logOutput.scrollTop = this.logOutput.scrollHeight; // Auto-scroll
        };
        state.logSocket.onclose = () => this.logOutput.innerHTML += '\n<span style="color: var(--color-error)">[Log stream disconnected.]</span>';
    },

    hideLogModal() {
        this.logModal.style.display = 'none';
        if (state.logSocket) {
            state.logSocket.close();
            state.logSocket = null;
        }
    }
};

document.getElementById('log-modal-close').onclick = () => ui.hideLogModal();

// --- Global Actions ---
window.nexus = {
    async serviceAction(id, action) {
        if(!confirm(`Confirm: ${action.toUpperCase()} service ${id}?`)) return;
        try {
            await fetch(`/api/service/${id}/${action}`, { method: 'POST' });
            // The main WS will update the UI state automatically
        } catch(e) { alert('Error: ' + e); }
    },

    async updateSvc(name) {
        if(!confirm(`RECREATE ${name}? This will pull the latest image and restart the container.`)) return;
        try {
            await fetch(`/api/update?service=${name}`, { method: 'POST' });
        } catch(e) { alert('Error: ' + e); }
    },

    async toggleAP(service, enabled) {
        await fetch('/api/toggle-autopilot', { 
            method: 'POST', 
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({service, enabled})
        });
    },

    showLogs(id, name) {
        ui.showLogModal(id, name);
    }
};

// --- Init ---
new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    if (msg.type === 'services_update') {
        state.services = msg.data;
        ui.render();
    }
    if (msg.type === 'node_update') {
        document.getElementById('node-name').innerText = msg.data.name.toUpperCase();
    }
}).connect();