import { WebSocketStream } from './websocket.js';

const state = {
    services: [],
    host: { cpu: 0, ram_used: 0, ram_total: 0, gpu: 0 },
    logSocket: null,
};

const ui = {
    grid: document.getElementById('services-grid'),
    
    // Host Elements
    hostName: document.getElementById('host-name'),
    hostCpuVal: document.getElementById('host-cpu-val'),
    hostCpuBar: document.getElementById('host-cpu-bar'),
    hostRamVal: document.getElementById('host-ram-val'),
    hostRamBar: document.getElementById('host-ram-bar'),
    hostGpuVal: document.getElementById('host-gpu-val'),
    hostGpuBar: document.getElementById('host-gpu-bar'),
    connStatus: document.getElementById('conn-status'),
    
    // Log Modal
    logModal: document.getElementById('log-modal'),
    logOutput: document.getElementById('log-output'),
    logTitle: document.getElementById('log-modal-title'),

    renderHost() {
        const h = state.host;
        
        // CPU
        this.hostCpuVal.innerText = `${h.cpu_usage.toFixed(1)}%`;
        this.hostCpuBar.style.width = `${Math.min(h.cpu_usage, 100)}%`;
        
        // RAM
        const ramPct = (h.ram_used / h.ram_total) * 100;
        this.hostRamVal.innerText = `${(h.ram_used / 1024).toFixed(1)} / ${(h.ram_total / 1024).toFixed(1)} GB`;
        this.hostRamBar.style.width = `${Math.min(ramPct, 100)}%`;

        // GPU
        if (h.gpu_usage > 0) {
            document.getElementById('host-gpu-row').style.opacity = '1';
            this.hostGpuVal.innerText = `${h.gpu_usage.toFixed(1)}%`;
            this.hostGpuBar.style.width = `${Math.min(h.gpu_usage, 100)}%`;
        }
        
        this.hostName.innerText = h.name || 'UNKNOWN';
    },

    renderServices() {
        if (!state.services.length) return;
        
        const sorted = [...state.services].sort((a, b) => {
            // Sort: GPU first, then Running, then Name
            if (a.has_gpu && !b.has_gpu) return -1;
            if (!a.has_gpu && b.has_gpu) return 1;
            const aUp = a.status.toLowerCase().includes('up');
            const bUp = b.status.toLowerCase().includes('up');
            if (aUp && !bUp) return -1;
            if (!aUp && bUp) return 1;
            return a.name.localeCompare(b.name);
        });

        this.grid.innerHTML = sorted.map(svc => this.createCard(svc)).join('');
    },

    createCard(svc) {
        const isUp = svc.status.toLowerCase().includes('up');
        const statusClass = isUp ? 'status-up' : 'status-down';
        const statusDot = isUp ? 'up' : 'down';
        
        // CPU & RAM Bars for Card
        const cpuPct = Math.min(svc.cpu_usage, 100);
        // Visual scale for RAM (assuming 2GB visual max for individual container bar, just for UI)
        const ramPct = Math.min((svc.mem_usage / 2048) * 100, 100); 

        return `
            <div class="card ${statusClass}">
                <div class="card-header">
                    <div>
                        <span class="card-title">
                            ${svc.name}
                            ${svc.has_gpu ? '<span class="gpu-tag">GPU</span>' : ''}
                        </span>
                        <span class="card-subtitle">${svc.image.substring(0, 25)}...</span>
                    </div>
                    <div style="text-align:right">
                        <span class="status-dot ${statusDot}"></span>
                        <span style="font-size:10px; font-weight:bold; color: #fff">${isUp ? 'RUNNING' : 'STOPPED'}</span>
                    </div>
                </div>

                <div class="card-stats">
                    <div>
                        <div class="stat-label"><span>CPU</span><span>${svc.cpu_usage.toFixed(1)}%</span></div>
                        <div class="progress-track"><div class="progress-fill cpu" style="width: ${cpuPct}%"></div></div>
                    </div>
                    <div>
                        <div class="stat-label"><span>MEM</span><span>${svc.mem_usage} MB</span></div>
                        <div class="progress-track"><div class="progress-fill ram" style="width: ${ramPct}%"></div></div>
                    </div>
                </div>
                
                <div class="card-actions">
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'start')" ${isUp ? 'disabled' : ''}>▶</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'stop')" ${!isUp ? 'disabled' : ''}>■</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'restart')" ${!isUp ? 'disabled' : ''}>↻</button>
                    <button style="flex:1" onclick="window.nexus.showLogs('${svc.short_id}', '${svc.name}')">TERMINAL</button>
                    <button class="${svc.auto_pilot ? 'btn-primary' : ''}" onclick="window.nexus.toggleAP('${svc.name}', ${!svc.auto_pilot})" title="Auto-Pilot">
                        ${svc.auto_pilot ? 'AP: ON' : 'AP: OFF'}
                    </button>
                </div>
            </div>
        `;
    },

    showLogModal(id, name) {
        this.logTitle.innerText = `root@${name}:~# tail -f`;
        this.logOutput.innerHTML = '';
        this.logModal.style.display = 'flex';
        
        if (state.logSocket) state.logSocket.close();
        
        state.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        state.logSocket.onopen = () => this.logOutput.innerHTML += '<span style="color: var(--accent)">[SECURE UPLINK ESTABLISHED]</span>\n';
        state.logSocket.onmessage = (event) => {
            this.logOutput.innerHTML += event.data;
            this.logOutput.scrollTop = this.logOutput.scrollHeight;
        };
        state.logSocket.onclose = () => this.logOutput.innerHTML += '\n<span style="color: var(--danger)">[UPLINK TERMINATED]</span>';
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
        // Simple confirmation
        if(action === 'stop' && !confirm('Stop service?')) return;
        try {
            await fetch(`/api/service/${id}/${action}`, { method: 'POST' });
        } catch(e) { alert('Error: ' + e); }
    },
    async toggleAP(service, enabled) {
        await fetch('/api/toggle-autopilot', { 
            method: 'POST', 
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({service, enabled})
        });
    },
    showLogs(id, name) { ui.showLogModal(id, name); }
};

// --- Connection ---
new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    ui.connStatus.innerText = "● LIVE UPLINK";
    ui.connStatus.style.color = "var(--accent)";
    ui.connStatus.style.borderColor = "var(--accent)";

    if (msg.type === 'services_update') {
        state.services = msg.data;
        ui.renderServices();
    }
    if (msg.type === 'node_update') {
        state.host = msg.data;
        ui.renderHost();
    }
}).connect();