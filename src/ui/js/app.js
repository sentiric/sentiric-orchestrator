// src/ui/js/app.js
import { WebSocketStream } from './websocket.js';
import { TopologyMap } from './components/topology.js';

const state = {
    cluster: {},
    selectedNode: null,
    localNodeName: null,
    logSocket: null,
    currentId: null
};

const ui = {
    grid: document.getElementById('services-grid'),
    clusterList: document.getElementById('cluster-list'),
    connStatus: document.getElementById('conn-status'),
    modal: document.getElementById('info-modal'),
    logView: document.getElementById('view-logs'),
    inspectView: document.getElementById('view-inspect'),
    inspectOutput: document.getElementById('inspect-output'),
    logTitle: document.getElementById('log-modal-title'),
    
    // View Switcher
    btnViewGrid: document.getElementById('btn-view-grid'),
    btnViewTopology: document.getElementById('btn-view-topology'),
    viewGrid: document.getElementById('view-grid'),
    viewTopology: document.getElementById('view-topology'),

    topology: null,

    init() {
        this.topology = new TopologyMap('topology-network');
        
        // Tab Events
        this.btnViewGrid.onclick = () => {
            this.btnViewGrid.classList.add('active');
            this.btnViewTopology.classList.remove('active');
            this.viewGrid.style.display = 'block';
            this.viewTopology.style.display = 'none';
        };

        this.btnViewTopology.onclick = () => {
            this.btnViewTopology.classList.add('active');
            this.btnViewGrid.classList.remove('active');
            this.viewGrid.style.display = 'none';
            this.viewTopology.style.display = 'block';
            this.topology.draw(); // Canvas'ı renderla
        };
    },

    renderSidebar() {
        if (!state.cluster) return;
        const nodes = Object.keys(state.cluster).sort();
        if (!state.selectedNode && nodes.length > 0) {
            state.selectedNode = nodes[0];
            state.localNodeName = nodes[0];
        }

        let html = '';
        nodes.forEach(nodeName => {
            const data = state.cluster[nodeName];
            const isActive = nodeName === state.selectedNode ? 'active' : '';
            const cpu = data.stats.cpu_usage.toFixed(0);
            const ram = Math.round((data.stats.ram_used / data.stats.ram_total) * 100) || 0;

            html += `
                <div class="node-item ${isActive}" onclick="window.nexus.selectNode('${nodeName}')">
                    <div class="node-item-header">
                        <span>💠 ${nodeName}</span>
                        <span style="color:${data.stats.status === 'ONLINE' ? 'var(--accent)' : 'var(--danger)'}">●</span>
                    </div>
                    <div class="node-mini-stats">
                        <div class="mini-bar"><div style="width:${cpu}%" class="bg-cpu"></div></div>
                        <div class="mini-bar"><div style="width:${ram}%" class="bg-ram"></div></div>
                    </div>
                </div>
            `;
        });
        this.clusterList.innerHTML = html;
        this.renderSelectedNode();
    },

    renderSelectedNode() {
        if (!state.selectedNode || !state.cluster[state.selectedNode]) return;
        const data = state.cluster[state.selectedNode];
        const h = data.stats;
        const services = data.services;

        document.getElementById('host-name').innerText = h.name;
        document.getElementById('host-cpu-val').innerText = `${h.cpu_usage.toFixed(1)}%`;
        document.getElementById('host-cpu-bar').style.width = `${Math.min(h.cpu_usage, 100)}%`;
        const ramPct = (h.ram_used / h.ram_total) * 100;
        document.getElementById('host-ram-val').innerText = `${(h.ram_used/1024).toFixed(1)} GB`;
        document.getElementById('host-ram-bar').style.width = `${Math.min(ramPct, 100)}%`;

        // Render Grid
        const sorted = [...services].sort((a, b) => a.name.localeCompare(b.name));
        this.grid.innerHTML = sorted.map(svc => this.createCard(svc)).join('');

        // YENİ: Topolojiyi Live Data ile Besle (Renkleri Güncelle)
        this.topology.updateLiveState(services);
    },

    createCard(svc) {
        const isUp = svc.status.toLowerCase().includes('up');
        const statusClass = isUp ? 'status-up' : 'status-down';
        const cpuPct = Math.min(svc.cpu_usage, 100);
        const ramPct = Math.min((svc.mem_usage / 2048) * 100, 100); 

        const actions = `
            <button onclick="window.nexus.serviceAction('${svc.short_id}', 'start')" ${isUp?'disabled':''}>▶</button>
            <button onclick="window.nexus.serviceAction('${svc.short_id}', 'stop')" ${!isUp?'disabled':''}>■</button>
            <button onclick="window.nexus.serviceAction('${svc.short_id}', 'restart')" ${!isUp?'disabled':''}>↻</button>
            <button onclick="window.nexus.openModal('${svc.short_id}', '${svc.name}')" style="flex:2">INFO</button>
            <button class="${svc.auto_pilot?'btn-primary':''}" onclick="window.nexus.toggleAP('${svc.name}', ${!svc.auto_pilot})">AP</button>
        `;

        return `
            <div class="card ${statusClass}">
                <div class="card-header">
                    <div>
                        <span class="card-title">${svc.name}</span>
                        <span class="card-subtitle">${svc.image.substring(0, 25)}...</span>
                    </div>
                    <div><span style="font-size:10px; font-weight:bold; color:#fff">${isUp?'RUNNING':'STOPPED'}</span></div>
                </div>
                <div class="card-stats">
                    <div><div class="progress-track"><div class="progress-fill cpu" style="width:${cpuPct}%"></div></div></div>
                    <div><div class="progress-track"><div class="progress-fill ram" style="width:${ramPct}%"></div></div></div>
                </div>
                <div class="card-actions">${actions}</div>
            </div>
        `;
    },

    openModal(id, name) {
        state.currentId = id;
        this.logTitle.innerText = `root@${name}:~#`;
        this.modal.style.display = 'flex';
        this.switchTab('logs');
    },

    async switchTab(tab) {
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
        if(tab === 'logs') {
            document.querySelector('.modal-tabs button[onclick*="logs"]').classList.add('active');
            this.logView.style.display = 'flex';
            this.inspectView.style.display = 'none';
            this.startLogStream(state.currentId);
        } else {
            document.querySelector('.modal-tabs button[onclick*="inspect"]').classList.add('active');
            this.logView.style.display = 'none';
            this.inspectView.style.display = 'flex';
            this.loadInspect(state.currentId);
        }
    },

    startLogStream(id) {
        this.logView.innerHTML = '<div id="log-output" class="log-container"></div>';
        const logOutput = document.getElementById('log-output');
        if (state.logSocket) state.logSocket.close();
        state.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        state.logSocket.onmessage = (e) => {
            logOutput.innerHTML += e.data;
            this.logView.scrollTop = this.logView.scrollHeight;
        };
    },

    async loadInspect(id) {
        this.inspectOutput.innerText = "Scanning...";
        try {
            const res = await fetch(`/api/service/${id}/inspect`);
            const data = await res.json();
            const clean = {
                Id: data.Id, Created: data.Created, State: data.State,
                Config: { Image: data.Config.Image, Env: data.Config.Env },
                Network: data.NetworkSettings.Networks
            };
            this.inspectOutput.innerText = JSON.stringify(clean, null, 2);
        } catch(e) { this.inspectOutput.innerText = "Error: " + e; }
    },

    hideModal() {
        this.modal.style.display = 'none';
        if (state.logSocket) state.logSocket.close();
    }
};

window.nexus = {
    selectNode(name) { state.selectedNode = name; ui.renderSidebar(); },
    serviceAction: async (id, act) => { if(act==='stop' && !confirm('Stop?')) return; await fetch(`/api/service/${id}/${act}`, {method:'POST'}); },
    toggleAP: async (svc, en) => { await fetch('/api/toggle-autopilot', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({service:svc, enabled:en})}); },
    openModal: (id, n) => ui.openModal(id, n),
    switchTab: (t) => ui.switchTab(t),
    hideModal: () => ui.hideModal(),
    pruneSystem: async () => { if(confirm('Prune system?')) await fetch('/api/system/prune', {method:'POST'}); },
    exportLLM: async () => {
        const res = await fetch('/api/export/llm');
        const text = await res.text();
        const a = document.createElement('a');
        a.href = window.URL.createObjectURL(new Blob([text], {type:'text/markdown'}));
        a.download = `nexus_report_${Date.now()}.md`;
        a.click();
    }
};

// --- BOOT SEQUENCE ---
document.getElementById('log-modal-close').onclick = () => ui.hideModal();
document.getElementById('btn-prune').onclick = () => window.nexus.pruneSystem();
document.getElementById('btn-export').onclick = () => window.nexus.exportLLM();

ui.init(); // Topoloji başlatılır

new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    ui.connStatus.innerText = "● CLUSTER LINK ACTIVE";
    ui.connStatus.style.color = "var(--accent)";
    
    if (msg.type === 'cluster_update') {
        state.cluster = msg.data;
        if (!state.localNodeName && Object.keys(state.cluster).length > 0) {
            state.localNodeName = Object.keys(state.cluster)[0];
        }
        ui.renderSidebar();
    }
}).connect();