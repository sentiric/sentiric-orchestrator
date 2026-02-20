import { WebSocketStream } from './websocket.js';

const state = {
    services: [],
    host: { cpu: 0, ram_used: 0, ram_total: 0, gpu: 0 },
    logSocket: null,
    currentId: null,
    startTime: Date.now()
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
    hostUptime: document.getElementById('host-uptime'),
    connStatus: document.getElementById('conn-status'),
    
    // Modal Elements
    modal: document.getElementById('info-modal'),
    logView: document.getElementById('view-logs'),
    inspectView: document.getElementById('view-inspect'),
    inspectOutput: document.getElementById('inspect-output'),
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

        // Uptime (Basit JS sayacı, backend timestamp göndermediği sürece)
        const seconds = Math.floor((Date.now() - state.startTime) / 1000);
        const hrs = Math.floor(seconds / 3600);
        const min = Math.floor((seconds % 3600) / 60);
        this.hostUptime.innerText = `${hrs}h ${min}m (Session)`;
    },

    renderServices() {
        if (!state.services.length) return;
        
        const sorted = [...state.services].sort((a, b) => {
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
        const cpuPct = Math.min(svc.cpu_usage, 100);
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
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'start')" ${isUp ? 'disabled' : ''} title="Start">▶</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'stop')" ${!isUp ? 'disabled' : ''} title="Stop">■</button>
                    <button onclick="window.nexus.serviceAction('${svc.short_id}', 'restart')" ${!isUp ? 'disabled' : ''} title="Restart">↻</button>
                    <button onclick="window.nexus.openModal('${svc.short_id}', '${svc.name}')" style="flex:2; border-color: #444; color: #fff">TERMINAL / X-RAY</button>
                    <button class="${svc.auto_pilot ? 'btn-primary' : ''}" onclick="window.nexus.toggleAP('${svc.name}', ${!svc.auto_pilot})" title="Auto-Pilot">
                        ${svc.auto_pilot ? 'AP' : 'AP'}
                    </button>
                </div>
            </div>
        `;
    },

    // --- MODAL & TABS ---
    openModal(id, name) {
        state.currentId = id;
        this.logTitle.innerText = `root@${name}:~#`;
        this.modal.style.display = 'flex';
        this.switchTab('logs');
    },

    async switchTab(tab) {
        document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
        if(tab === 'logs') {
            document.querySelector('button[onclick*="logs"]').classList.add('active');
            this.logView.style.display = 'flex';
            this.inspectView.style.display = 'none';
            this.startLogStream(state.currentId);
        } else {
            document.querySelector('button[onclick*="inspect"]').classList.add('active');
            this.logView.style.display = 'none';
            this.inspectView.style.display = 'flex';
            this.loadInspect(state.currentId);
        }
    },

    startLogStream(id) {
        this.logView.innerHTML = '';
        if (state.logSocket) state.logSocket.close();
        
        state.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        state.logSocket.onopen = () => this.logView.innerHTML += '<span style="color: var(--accent)">[SECURE UPLINK ESTABLISHED]</span>\n';
        state.logSocket.onmessage = (e) => {
            this.logView.innerHTML += e.data;
            this.logView.scrollTop = this.logView.scrollHeight;
        };
        state.logSocket.onclose = () => this.logView.innerHTML += '\n<span style="color: var(--danger)">[UPLINK TERMINATED]</span>';
    },

    async loadInspect(id) {
        this.inspectOutput.innerText = "Scanning container structure...";
        try {
            const res = await fetch(`/api/service/${id}/inspect`);
            const data = await res.json();
            
            const clean = {
                Id: data.Id,
                Created: data.Created,
                State: data.State,
                Image: data.Config.Image,
                CMD: data.Config.Cmd,
                Env: data.Config.Env,
                Mounts: data.Mounts,
                Network: data.NetworkSettings.Networks,
                Ports: data.NetworkSettings.Ports
            };
            this.inspectOutput.innerText = JSON.stringify(clean, null, 2);
        } catch(e) {
            this.inspectOutput.innerText = "Error: " + e;
        }
    },

    hideModal() {
        this.modal.style.display = 'none';
        if (state.logSocket) state.logSocket.close();
    }
};

window.nexus = {
    serviceAction: async (id, action) => {
         if(action === 'stop' && !confirm('Stop service?')) return;
         await fetch(`/api/service/${id}/${action}`, { method: 'POST' });
    },
    toggleAP: async (service, enabled) => {
        await fetch('/api/toggle-autopilot', { method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify({service, enabled}) });
    },
    openModal: (id, name) => ui.openModal(id, name),
    switchTab: (tab) => ui.switchTab(tab),
    hideModal: () => ui.hideModal(),
    
    // --- ACTIONS ---
    pruneSystem: async () => {
        if(!confirm('☢ WARNING: This will remove all stopped containers and unused images. Continue?')) return;
        try {
            const res = await fetch('/api/system/prune', { method: 'POST' });
            alert(await res.text());
        } catch(e) { alert(e); }
    },
    
    exportLLM: async () => {
        const btn = document.getElementById('btn-export');
        btn.innerText = "GENERATING...";
        try {
            const res = await fetch('/api/export/llm');
            const text = await res.text();
            
            // Download File
            const blob = new Blob([text], { type: 'text/markdown' });
            const url = window.URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = `sentiric_report_${Date.now()}.md`;
            a.click();
            btn.innerText = "🤖 AI EXPORT";
        } catch(e) { 
            alert(e); 
            btn.innerText = "🤖 AI EXPORT";
        }
    }
};

document.getElementById('log-modal-close').onclick = () => ui.hideModal();
document.getElementById('btn-prune').onclick = () => window.nexus.pruneSystem();
document.getElementById('btn-export').onclick = () => window.nexus.exportLLM();

new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    ui.connStatus.innerText = "● LIVE UPLINK";
    ui.connStatus.style.color = "var(--accent)";
    if (msg.type === 'services_update') { state.services = msg.data; ui.renderServices(); }
    if (msg.type === 'node_update') { state.host = msg.data; ui.renderHost(); }
}).connect();