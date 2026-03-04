// src/ui/js/app.js
import { WebSocketStream } from './websocket.js';

// --- 1. STATE STORE (Hafıza) ---
const Store = {
    state: {
        cluster: {},
        selectedNode: null,
        localNodeName: null,
        history: {} // Sparkline verisi için son 30 ölçüm
    },
    listeners: [],
    subscribe(callback) { this.listeners.push(callback); },
    dispatch(actionType, payload) {
        switch(actionType) {
            case 'CLUSTER_UPDATE':
                this.state.cluster = payload;
                if (!this.state.localNodeName && Object.keys(this.state.cluster).length > 0) {
                    this.state.localNodeName = Object.keys(this.state.cluster)[0];
                }
                this.updateHistory(payload);
                this.notify();
                break;
            case 'SELECT_NODE':
                this.state.selectedNode = payload;
                this.notify();
                break;
            case 'TOGGLE_AP_OPTIMISTIC':
                const { node, service, enabled } = payload;
                if (this.state.cluster[node]) {
                    const svc = this.state.cluster[node].services.find(s => s.name === service);
                    if (svc) svc.auto_pilot = enabled;
                }
                this.notify();
                break;
        }
    },
    updateHistory(clusterData) {
        const MAX_HISTORY = 30;
        Object.keys(clusterData).forEach(nodeName => {
            const services = clusterData[nodeName].services;
            services.forEach(svc => {
                const id = `${nodeName}_${svc.name}`;
                if (!this.state.history[id]) {
                    this.state.history[id] = { cpu: new Array(MAX_HISTORY).fill(0), ram: new Array(MAX_HISTORY).fill(0) };
                }
                this.state.history[id].cpu.push(svc.cpu_usage);
                this.state.history[id].cpu.shift();
                
                this.state.history[id].ram.push(svc.mem_usage);
                this.state.history[id].ram.shift();
            });
        });
    },
    notify() { this.listeners.forEach(fn => fn(this.state)); }
};

// --- 2. TOPOLOGY MAP (Vis.js Wrapper) ---
class TopologyMap {
    constructor(containerId) {
        this.container = document.getElementById(containerId);
        this.network = null;
        this.nodes = typeof vis !== 'undefined' ? new vis.DataSet() : null;
        this.edges = typeof vis !== 'undefined' ? new vis.DataSet() : null;
        this.isDrawn = false;

        this.colors = {
            expected: { bg: '#161b22', border: '#30363d', font: '#666' },
            healthy: { bg: 'rgba(0, 255, 157, 0.1)', border: '#00ff9d', font: '#fff' },
            warning: { bg: 'rgba(210, 153, 34, 0.1)', border: '#d29922', font: '#fff' },
            critical: { bg: 'rgba(248, 81, 73, 0.1)', border: '#f85149', font: '#fff' }
        };
    }

    async draw() {
        if (this.isDrawn || !this.nodes) return;

        try {
            const res = await fetch('/api/topology');
            const data = await res.json();

            const visNodes = data.nodes.map(n => ({
                id: n.id, label: n.label, group: n.group, shape: 'box',
                color: { background: this.colors.expected.bg, border: this.colors.expected.border },
                font: { color: this.colors.expected.font, face: 'JetBrains Mono', size: 12 },
                borderWidth: 2, shadow: true
            }));

            const visEdges = data.edges.map(e => ({
                from: e.from, to: e.to, label: e.label,
                font: { align: 'middle', color: '#666', size: 10, face: 'Inter' },
                arrows: 'to', color: { color: '#333', highlight: '#00ff9d' }, dashes: e.dashes
            }));

            this.nodes.add(visNodes);
            this.edges.add(visEdges);

            const options = {
                layout: { hierarchical: { direction: 'UD', sortMethod: 'directed', nodeSpacing: 150, levelSeparation: 150 } },
                physics: false,
                interaction: { hover: true, zoomView: true, dragView: true }
            };

            this.network = new vis.Network(this.container, { nodes: this.nodes, edges: this.edges }, options);
            this.isDrawn = true;
        } catch (e) {
            console.error("Failed to load topology:", e);
        }
    }

    updateLiveState(liveServices) {
        if (!this.isDrawn || !this.nodes) return;

        const updates = [];
        this.nodes.get().forEach(node => {
            const liveSvc = liveServices.find(s => s.name === node.id);
            let newColor = this.colors.expected;

            if (liveSvc) {
                const isUp = liveSvc.status.toLowerCase().includes('up');
                if (isUp) {
                    if (liveSvc.cpu_usage > 80 || (liveSvc.mem_usage > 2000)) newColor = this.colors.warning;
                    else newColor = this.colors.healthy;
                } else {
                    newColor = this.colors.critical;
                }
            } else {
                 newColor = this.colors.critical;
            }

            if (node.color.border !== newColor.border) {
                updates.push({ id: node.id, color: { background: newColor.bg, border: newColor.border }, font: { color: newColor.font } });
            }
        });

        if (updates.length > 0) this.nodes.update(updates);
    }
}

// --- 3. UI KONTROLCÜSÜ (DOM Manipülasyonu) ---
const ui = {
    grid: document.getElementById('services-grid'),
    clusterList: document.getElementById('cluster-list'),
    connStatus: document.getElementById('conn-status'),
    modal: document.getElementById('info-modal'),
    logView: document.getElementById('view-logs'),
    inspectView: document.getElementById('view-inspect'),
    inspectOutput: document.getElementById('inspect-output'),
    logTitle: document.getElementById('log-modal-title'),
    btnViewGrid: document.getElementById('btn-view-grid'),
    btnViewTopology: document.getElementById('btn-view-topology'),
    viewGrid: document.getElementById('view-grid'),
    viewTopology: document.getElementById('view-topology'),

    topology: null,
    logSocket: null,
    currentId: null,

    init() {
        this.topology = new TopologyMap('topology-network');
        
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
            this.topology.draw(); 
        };

        Store.subscribe((state) => {
            requestAnimationFrame(() => {
                this.renderSidebar(state);
                this.renderSelectedNode(state);
            });
        });
    },

    renderSidebar(state) {
        if (!state.cluster) return;
        const nodes = Object.keys(state.cluster).sort();
        if (!state.selectedNode && nodes.length > 0) {
            Store.dispatch('SELECT_NODE', nodes[0]);
            return;
        }

        let html = '';
        nodes.forEach(nodeName => {
            const data = state.cluster[nodeName];
            const isActive = nodeName === state.selectedNode ? 'active' : '';
            const cpu = data.stats.cpu_usage.toFixed(0);
            const ram = Math.round((data.stats.ram_used / data.stats.ram_total) * 100) || 0;

            html += `
                <div class="node-item ${isActive}" onclick="Store.dispatch('SELECT_NODE', '${nodeName}')">
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
    },

    renderSelectedNode(state) {
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

        const sorted = [...services].sort((a, b) => a.name.localeCompare(b.name));
        this.grid.innerHTML = sorted.map(svc => this.createCard(svc, state)).join('');
        
        // Sparklines Render (Canvas)
        sorted.forEach(svc => {
            const hist = state.history[`${state.selectedNode}_${svc.name}`];
            if (hist) {
                this.drawSparkline(`cvs-cpu-${svc.short_id}`, hist.cpu, '#0984e3');
                this.drawSparkline(`cvs-ram-${svc.short_id}`, hist.ram, '#00ff9d');
            }
        });

        this.topology.updateLiveState(services);
    },

    drawSparkline(canvasId, dataArray, color) {
        const canvas = document.getElementById(canvasId);
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width;
        const h = canvas.height;
        
        ctx.clearRect(0, 0, w, h);
        ctx.beginPath();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;

        let max = Math.max(...dataArray, 10);
        const step = w / (dataArray.length - 1);
        
        dataArray.forEach((val, i) => {
            const y = h - ((val / max) * h);
            if (i === 0) ctx.moveTo(0, y);
            else ctx.lineTo(i * step, y);
        });
        ctx.stroke();
    },

    createCard(svc, state) {
        const isUp = svc.status.toLowerCase().includes('up');
        const statusClass = isUp ? 'status-up' : 'status-down';
        const cpuPct = Math.min(svc.cpu_usage, 100);
        const ramPct = Math.min((svc.mem_usage / 2048) * 100, 100); 
        const isRemote = state.selectedNode !== state.localNodeName;
        
        let actions = '';
        if (isRemote) {
            actions = `<button onclick="window.open('http://${state.selectedNode}:11080', '_blank')" style="flex:1; border-color:var(--accent); color:var(--accent)">↗ OPEN REMOTE DASHBOARD</button>`;
        } else {
            const apClick = `
                Store.dispatch('TOGGLE_AP_OPTIMISTIC', { node: '${state.selectedNode}', service: '${svc.name}', enabled: ${!svc.auto_pilot} });
                fetch('/api/toggle-autopilot', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({service:'${svc.name}', enabled:${!svc.auto_pilot}})});
            `;
            actions = `
                <button onclick="fetch('/api/service/${svc.short_id}/start', {method:'POST'})" ${isUp?'disabled':''}>▶</button>
                <button onclick="if(confirm('Stop?')) fetch('/api/service/${svc.short_id}/stop', {method:'POST'})" ${!isUp?'disabled':''}>■</button>
                <button onclick="fetch('/api/service/${svc.short_id}/restart', {method:'POST'})" ${!isUp?'disabled':''}>↻</button>
                <button onclick="ui.openModal('${svc.short_id}', '${svc.name}')" style="flex:2">INFO</button>
                <button class="${svc.auto_pilot?'btn-primary':''}" onclick="${apClick.replace(/\n/g, '')}">AP</button>
            `;
        }

        return `
            <div class="card ${statusClass}">
                <div class="card-header">
                    <div>
                        <span class="card-title">${svc.name} ${svc.has_gpu ? '<span class="gpu-tag">GPU</span>' : ''}</span>
                        <span class="card-subtitle">${svc.image.substring(0, 25)}...</span>
                    </div>
                    <div><span style="font-size:10px; font-weight:bold; color:#fff">${isUp?'RUNNING':'STOPPED'}</span></div>
                </div>
                <div class="card-stats">
                    <div>
                        <div class="progress-track"><div class="progress-fill cpu" style="width:${cpuPct}%"></div></div>
                        <div class="sparkline-container"><canvas id="cvs-cpu-${svc.short_id}" class="sparkline-canvas" width="100" height="16"></canvas></div>
                    </div>
                    <div>
                        <div class="progress-track"><div class="progress-fill ram" style="width:${ramPct}%"></div></div>
                        <div class="sparkline-container"><canvas id="cvs-ram-${svc.short_id}" class="sparkline-canvas" width="100" height="16"></canvas></div>
                    </div>
                </div>
                <div class="card-actions">${actions}</div>
            </div>
        `;
    },

    openModal(id, name) {
        this.currentId = id;
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
            this.startLogStream(this.currentId);
        } else {
            document.querySelector('.modal-tabs button[onclick*="inspect"]').classList.add('active');
            this.logView.style.display = 'none';
            this.inspectView.style.display = 'flex';
            this.loadInspect(this.currentId);
        }
    },

    startLogStream(id) {
        this.logView.innerHTML = '<div id="log-output" class="log-container"></div>';
        const logOutput = document.getElementById('log-output');
        if (this.logSocket) this.logSocket.close();
        this.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        this.logSocket.onmessage = (e) => {
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
        if (this.logSocket) this.logSocket.close();
    }
};

window.Store = Store; 
window.ui = ui;

document.getElementById('log-modal-close').onclick = () => ui.hideModal();
document.getElementById('btn-prune').onclick = async () => {
    if(confirm('Prune system?')) await fetch('/api/system/prune', {method:'POST'});
};
document.getElementById('btn-export').onclick = async () => {
    const res = await fetch('/api/export/llm');
    const text = await res.text();
    const a = document.createElement('a');
    a.href = window.URL.createObjectURL(new Blob([text], {type:'text/markdown'}));
    a.download = `nexus_report_${Date.now()}.md`;
    a.click();
};

// --- BOOT SEQUENCE ---
ui.init(); 

new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    ui.connStatus.innerText = "● CLUSTER LINK ACTIVE";
    ui.connStatus.style.color = "var(--accent)";
    
    if (msg.type === 'cluster_update') {
        Store.dispatch('CLUSTER_UPDATE', msg.data);
    }
}).connect();