// src/ui/js/app.js
import { WebSocketStream } from './websocket.js';
import { TopologyMap } from './components/topology.js';
import { Store } from './store.js';

const ui = {
    grid: document.getElementById('services-grid'),
    clusterList: document.getElementById('cluster-list'),
    connStatus: document.getElementById('conn-status'),
    modal: document.getElementById('info-modal'),
    logView: document.getElementById('view-logs'),
    inspectView: document.getElementById('view-inspect'),
    violationsView: document.getElementById('view-violations'),
    inspectOutput: document.getElementById('inspect-output'),
    violationsOutput: document.getElementById('violations-output'),
    
    // View Buttons
    btnViewGrid: document.getElementById('btn-view-grid'),
    btnViewTopology: document.getElementById('btn-view-topology'),
    btnViewGridMobile: document.getElementById('btn-view-grid-mobile'),
    btnViewTopologyMobile: document.getElementById('btn-view-topology-mobile'),
    
    viewGrid: document.getElementById('view-grid'),
    viewTopology: document.getElementById('view-topology'),

    topology: null,
    logSocket: null,
    currentId: null,

    init() {
        this.topology = new TopologyMap('topology-network');
        this.bindEvents();
        
        Store.subscribe((state) => {
            requestAnimationFrame(() => {
                this.renderSidebar(state);
                this.renderSelectedNode(state);
                
                if (this.topology.isDrawn && this.viewTopology.classList.contains('active')) {
                    this.topology.updateLiveState(state.cluster);
                }
            });
        });
    },

    bindEvents() {
        // --- Mobil Menü ---
        const btnMenu = document.getElementById('mobile-menu-btn');
        if (btnMenu) {
            btnMenu.onclick = () => {
                document.getElementById('sidebar').classList.toggle('open');
            };
        }

        // Kapatma işlemi (Dışarı tıklayınca menü kapansın)
        document.addEventListener('click', (e) => {
            const sidebar = document.getElementById('sidebar');
            if (window.innerWidth <= 768 && sidebar && sidebar.classList.contains('open')) {
                if (!sidebar.contains(e.target) && e.target.id !== 'mobile-menu-btn') {
                    sidebar.classList.remove('open');
                }
            }
        });

        // --- View Toggles ---
        const switchToGrid = () => {
            if(this.btnViewGrid) this.btnViewGrid.classList.add('active');
            if(this.btnViewTopology) this.btnViewTopology.classList.remove('active');
            if(this.btnViewGridMobile) this.btnViewGridMobile.classList.add('active');
            if(this.btnViewTopologyMobile) this.btnViewTopologyMobile.classList.remove('active');
            this.viewGrid.classList.add('active');
            this.viewTopology.classList.remove('active');
        };

        const switchToMap = () => {
            if(this.btnViewTopology) this.btnViewTopology.classList.add('active');
            if(this.btnViewGrid) this.btnViewGrid.classList.remove('active');
            if(this.btnViewTopologyMobile) this.btnViewTopologyMobile.classList.add('active');
            if(this.btnViewGridMobile) this.btnViewGridMobile.classList.remove('active');
            this.viewGrid.classList.remove('active');
            this.viewTopology.classList.add('active');
            this.topology.draw().then(() => this.topology.updateLiveState(Store.state.cluster));
        };

        if(this.btnViewGrid) this.btnViewGrid.onclick = switchToGrid;
        if(this.btnViewGridMobile) this.btnViewGridMobile.onclick = switchToGrid;
        if(this.btnViewTopology) this.btnViewTopology.onclick = switchToMap;
        if(this.btnViewTopologyMobile) this.btnViewTopologyMobile.onclick = switchToMap;

        // --- Modals ---
        document.querySelectorAll('.modal-close').forEach(btn => btn.onclick = () => this.hideModal());
        
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(btn => {
            btn.onclick = (e) => {
                document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
                document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));
                
                e.target.classList.add('active');
                const targetId = e.target.getAttribute('data-target');
                document.getElementById(targetId).classList.add('active');

                if (targetId === 'view-logs') this.startLogStream(this.currentId);
                if (targetId === 'view-inspect') this.loadInspect(this.currentId);
            };
        });

        document.getElementById('btn-prune').onclick = async () => {
            if(confirm('🗑️ WARNING: This will prune stopped containers and dangling images. Proceed?')) {
                await fetch('/api/system/prune', {method:'POST'});
            }
        };

        document.getElementById('btn-export').onclick = async () => {
            const res = await fetch('/api/export/llm');
            let text = await res.text();
            
            const a = document.createElement('a');
            a.href = window.URL.createObjectURL(new Blob([text], {type:'text/markdown'}));
            a.download = `nexus_diagnostic_${Date.now()}.md`;
            a.click();
        };

        // Grid içi buton dinlemeleri (Delegation)
        this.grid.addEventListener('click', (e) => {
            const btnInfo = e.target.closest('.btn-info');
            if (btnInfo) {
                this.openModal(btnInfo.dataset.id, btnInfo.dataset.name);
                return;
            }
            
            const btnViolations = e.target.closest('.badge-quarantine');
            if (btnViolations) {
                const nodeName = Store.state.selectedNode;
                const svcName = btnViolations.dataset.name;
                const nodeData = Store.state.cluster[nodeName];
                if (nodeData) {
                    const svc = nodeData.services.find(s => s.name === svcName);
                    if (svc) this.openViolationsModal(svc);
                }
                return;
            }
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
                <div class="node-item ${isActive}" onclick="window.Store.dispatch('SELECT_NODE', '${nodeName}')">
                    <div class="node-item-head">
                        <span class="node-item-name">💠 ${nodeName}</span>
                        <div class="node-status-dot ${data.stats.status === 'ONLINE' ? 'online' : 'offline'}"></div>
                    </div>
                    <div class="mini-stats">
                        <div class="mini-stat-bar"><div style="width:${cpu}%" class="mini-stat-fill cpu"></div></div>
                        <div class="mini-stat-bar"><div style="width:${ram}%" class="mini-stat-fill ram"></div></div>
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
        
        sorted.forEach(svc => {
            const hist = state.history[`${state.selectedNode}_${svc.name}`];
            if (hist) {
                this.drawSparkline(`cvs-cpu-${svc.short_id}`, hist.cpu, '#3b82f6', 'rgba(59, 130, 246, 0.15)');
                this.drawSparkline(`cvs-ram-${svc.short_id}`, hist.ram, '#10b981', 'rgba(16, 185, 129, 0.15)');
            }
        });
    },

    drawSparkline(canvasId, dataArray, lineColor, fillColor) {
        const canvas = document.getElementById(canvasId);
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const w = canvas.width; const h = canvas.height;
        ctx.clearRect(0, 0, w, h);
        let max = Math.max(...dataArray, 5); 
        const step = w / (dataArray.length - 1);
        
        ctx.beginPath();
        dataArray.forEach((val, i) => {
            const y = h - ((val / max) * h);
            if (i === 0) ctx.moveTo(0, y); else ctx.lineTo(i * step, y);
        });
        ctx.lineTo(w, h); ctx.lineTo(0, h); ctx.closePath();
        ctx.fillStyle = fillColor; ctx.fill();

        ctx.beginPath();
        dataArray.forEach((val, i) => {
            const y = h - ((val / max) * h);
            if (i === 0) ctx.moveTo(0, y); else ctx.lineTo(i * step, y);
        });
        ctx.strokeStyle = lineColor; ctx.lineWidth = 1.5; ctx.stroke();
    },

    createCard(svc, state) {
        // --- V6.0 HEALTH STATE RESOLUTION ---
        let statusClass = 'status-offline';
        let statusText = 'STOPPED';
        let badgesHtml = '';

        if (svc.health === 'Quarantined') {
            statusClass = 'status-quarantined';
            statusText = 'QUARANTINED';
            badgesHtml += `<span class="badge badge-quarantine" data-name="${svc.name}">⚠️ ${svc.violations.length} VIOLATIONS</span>`;
        } else if (svc.health === 'Draining') {
            statusClass = 'status-draining';
            statusText = 'DRAINING (UPDATING)';
            badgesHtml += `<span class="badge badge-draining">⏳ GRACEFUL SHUTDOWN</span>`;
        } else if (svc.health === 'RiskOom') {
            statusClass = 'status-riskoom';
            statusText = 'RUNNING (RISK)';
            badgesHtml += `<span class="badge badge-oom">🧠 RAM LIMIT</span>`;
        } else if (svc.health === 'Online') {
            statusClass = 'status-online';
            statusText = 'RUNNING';
        }

        if (svc.has_gpu) badgesHtml += `<span class="badge badge-gpu">GPU</span>`;

        const isUp = svc.status.toLowerCase().includes('up');
        const isRemote = state.selectedNode !== state.localNodeName;
        
        let actions = '';
        if (isRemote) {
            actions = `<button class="btn" onclick="window.open('http://${state.selectedNode}:11080', '_blank')" style="width:100%; border-color:var(--accent-blue); color:var(--accent-blue)">↗ OPEN NODE PORTAL</button>`;
        } else {
            const apClick = `
                window.Store.dispatch('TOGGLE_AP_OPTIMISTIC', { node: '${state.selectedNode}', service: '${svc.name}', enabled: ${!svc.auto_pilot} });
                fetch('/api/toggle-autopilot', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({service:'${svc.name}', enabled:${!svc.auto_pilot}})});
            `;
            
            // Eğer karantinadaysa veya draining ise butonları kilitle
            const btnDisabled = (svc.health === 'Quarantined' || svc.health === 'Draining' || !isUp) ? 'disabled' : '';

            actions = `
                <button class="btn" onclick="fetch('/api/service/${svc.short_id}/start', {method:'POST'})" ${isUp?'disabled':''}>▶</button>
                <button class="btn" onclick="if(confirm('Stop ${svc.name}?')) fetch('/api/service/${svc.short_id}/stop', {method:'POST'})" ${btnDisabled}>■</button>
                <button class="btn" onclick="fetch('/api/service/${svc.short_id}/restart', {method:'POST'})" ${btnDisabled}>↻</button>
                <button class="btn btn-info" data-id="${svc.short_id}" data-name="${svc.name}" style="flex:2">INFO</button>
                <button class="btn ${svc.auto_pilot?'btn-primary':''}" onclick="${apClick.replace(/\n/g, '')}">AP</button>
            `;
        }

        return `
            <div class="service-card ${statusClass}">
                <div class="svc-header">
                    <div>
                        <div class="svc-title">${svc.name} ${badgesHtml}</div>
                        <span class="svc-image">${svc.image.split('@')[0]}</span>
                    </div>
                    <div class="svc-status" style="color:var(--text-main);">${statusText}</div>
                </div>
                <div class="svc-metrics">
                    <div class="metric-row">
                        <div class="m-labels"><span class="lbl">CPU</span><span class="val cpu">${svc.cpu_usage.toFixed(1)}%</span></div>
                        <div class="sparkline-box"><canvas id="cvs-cpu-${svc.short_id}" class="sparkline-canvas" width="400" height="24"></canvas></div>
                    </div>
                    <div class="metric-row">
                        <div class="m-labels"><span class="lbl">RAM</span><span class="val ram">${svc.mem_usage} MB</span></div>
                        <div class="sparkline-box"><canvas id="cvs-ram-${svc.short_id}" class="sparkline-canvas" width="400" height="24"></canvas></div>
                    </div>
                </div>
                <div class="svc-actions">${actions}</div>
            </div>
        `;
    },

    openModal(id, name) {
        this.currentId = id;
        this.modal.style.display = 'flex';
        
        // Reset tabs
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
        document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));
        
        document.getElementById('tab-violations').style.display = 'none'; // Normalde gizle
        document.querySelector('.modal-tabs .tab-btn[data-target="view-logs"]').classList.add('active');
        document.getElementById('view-logs').classList.add('active');
        
        this.startLogStream(id);
    },

    openViolationsModal(svc) {
        this.modal.style.display = 'flex';
        
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
        document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));

        const vioTab = document.getElementById('tab-violations');
        vioTab.style.display = 'block';
        vioTab.classList.add('active');
        document.getElementById('view-violations').classList.add('active');

        let html = `<h3>🚫 QUARANTINE REPORT: ${svc.name}</h3><br>`;
        html += `<p>This service has been flagged by the Sentinel Governor for architectural compliance violations.</p><br><ul>`;
        svc.violations.forEach(v => {
            html += `<li style="margin-bottom:10px;">${v}</li>`;
        });
        html += `</ul><br><p style="color:#666;">Action Required: Fix the environment variables (.env) or Docker configurations and restart the service.</p>`;
        
        this.violationsOutput.innerHTML = html;
        if (this.logSocket) this.logSocket.close(); // Log akışını durdur
    },

    startLogStream(id) {
        const logOutput = document.getElementById('log-output');
        logOutput.innerHTML = '';
        if (this.logSocket) this.logSocket.close();
        
        this.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        this.logSocket.onmessage = (e) => {
            logOutput.innerHTML += e.data;
            this.logView.scrollTop = this.logView.scrollHeight;
        };
    },

    async loadInspect(id) {
        this.inspectOutput.innerText = "Scanning Docker API...";
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
    },
};

window.Store = Store; 
window.ui = ui;

ui.init(); 

new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    const st = document.getElementById('conn-status');
    if (st) {
        st.innerText = "● CLUSTER LINK ACTIVE";
        st.className = "conn-status online";
    }
    if (msg.type === 'cluster_update') {
        Store.dispatch('CLUSTER_UPDATE', msg.data);
    }
}, (isOnline) => {
    const st = document.getElementById('conn-status');
    if (st) {
        st.innerText = isOnline ? "● CLUSTER LINK ACTIVE" : "● OFFLINE";
        st.className = `conn-status ${isOnline ? 'online' : 'offline'}`;
    }
}).connect();