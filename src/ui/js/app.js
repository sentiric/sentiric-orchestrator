// src/ui/js/app.js
import { WebSocketStream } from './websocket.js';
import { TopologyMap } from './components/topology.js';
import { Store } from './store.js';

const ui = {
    // Referansları güvenli tutmak için getElementById yapıyoruz, ancak null ise çökmeyecek şekilde kurgulandı.
    grid: document.getElementById('services-grid'),
    clusterList: document.getElementById('cluster-list'),
    
    viewGrid: document.getElementById('view-grid'),
    viewTopology: document.getElementById('view-topology'),

    topology: null,
    logSocket: null,
    currentId: null,

    init() {
        console.log("💠 Sovereign Orchestrator UI Initializing...");

        // Topology Network başlat
        try {
            if (document.getElementById('topology-network')) {
                this.topology = new TopologyMap('topology-network');
            }
        } catch(e) {
            console.warn("Topology UI Map init skipped:", e);
        }
        
        this.bindEvents();
        
        Store.subscribe((state) => {
            requestAnimationFrame(() => {
                this.renderSidebar(state);
                this.renderSelectedNode(state);
                
                if (this.topology && this.topology.isDrawn && this.viewTopology && this.viewTopology.classList.contains('active')) {
                    this.topology.updateLiveState(state.cluster);
                }
            });
        });
    },

    // DEFENSIVE EVENT BINDING: Eğer element yoksa çökmez!
    safeClick(id, handler) {
        const el = document.getElementById(id);
        if (el) {
            el.addEventListener('click', handler);
        } else {
            console.warn(`[UI Warning] Element '#${id}' not found. Click event skipped.`);
        }
    },

    bindEvents() {
        // --- Mobil Menü ---
        this.safeClick('mobile-menu-btn', () => {
            const sidebar = document.getElementById('sidebar');
            if (sidebar) sidebar.classList.toggle('open');
        });

        // Mobil Menü Kapatma (Dışarı tıklayınca)
        document.addEventListener('click', (e) => {
            const sidebar = document.getElementById('sidebar');
            if (window.innerWidth <= 768 && sidebar && sidebar.classList.contains('open')) {
                if (!sidebar.contains(e.target) && e.target.id !== 'mobile-menu-btn') {
                    sidebar.classList.remove('open');
                }
            }
        });

        // --- Görünüm Geçişleri (View Toggles) ---
        const switchToGrid = () => {
            document.getElementById('btn-view-grid')?.classList.add('active');
            document.getElementById('btn-view-grid-mobile')?.classList.add('active');
            
            document.getElementById('btn-view-topology')?.classList.remove('active');
            document.getElementById('btn-view-topology-mobile')?.classList.remove('active');
            
            this.viewGrid?.classList.add('active');
            this.viewTopology?.classList.remove('active');
        };

        const switchToMap = () => {
            document.getElementById('btn-view-topology')?.classList.add('active');
            document.getElementById('btn-view-topology-mobile')?.classList.add('active');
            
            document.getElementById('btn-view-grid')?.classList.remove('active');
            document.getElementById('btn-view-grid-mobile')?.classList.remove('active');
            
            this.viewGrid?.classList.remove('active');
            this.viewTopology?.classList.add('active');
            
            if (this.topology) {
                this.topology.draw().then(() => this.topology.updateLiveState(Store.state.cluster));
            }
        };

        this.safeClick('btn-view-grid', switchToGrid);
        this.safeClick('btn-view-grid-mobile', switchToGrid);
        this.safeClick('btn-view-topology', switchToMap);
        this.safeClick('btn-view-topology-mobile', switchToMap);

        // --- Modals ---
        this.safeClick('btn-modal-close', () => this.hideModal());
        
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(btn => {
            btn.addEventListener('click', (e) => {
                document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
                document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));
                
                e.target.classList.add('active');
                const targetId = e.target.getAttribute('data-target');
                const targetView = document.getElementById(targetId);
                if(targetView) targetView.classList.add('active');

                if (targetId === 'view-logs') this.startLogStream(this.currentId);
                if (targetId === 'view-inspect') this.loadInspect(this.currentId);
            });
        });

        // --- System Actions ---
        this.safeClick('btn-prune', async () => {
            if(confirm('🗑️ WARNING: This will prune stopped containers and dangling images. Proceed?')) {
                try { await fetch('/api/system/prune', {method:'POST'}); } catch(e) { console.error(e); }
            }
        });

        this.safeClick('btn-export', async () => {
            try {
                const res = await fetch('/api/export/llm');
                const text = await res.text();
                const a = document.createElement('a');
                a.href = window.URL.createObjectURL(new Blob([text], {type:'text/markdown'}));
                a.download = `nexus_diagnostic_${Date.now()}.md`;
                a.click();
            } catch(e) { console.error("Export Failed", e); }
        });

        // --- Grid Actions (Delegation) ---
        if (this.grid) {
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
        }
    },

    renderSidebar(state) {
        if (!state.cluster || !this.clusterList) return;
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
        if (!state.selectedNode || !state.cluster[state.selectedNode] || !this.grid) return;
        const data = state.cluster[state.selectedNode];
        const h = data.stats;
        const services = data.services;

        const elHostName = document.getElementById('host-name');
        const elHostCpuVal = document.getElementById('host-cpu-val');
        const elHostCpuBar = document.getElementById('host-cpu-bar');
        const elHostRamVal = document.getElementById('host-ram-val');
        const elHostRamBar = document.getElementById('host-ram-bar');

        if(elHostName) elHostName.innerText = h.name;
        if(elHostCpuVal) elHostCpuVal.innerText = `${h.cpu_usage.toFixed(1)}%`;
        if(elHostCpuBar) elHostCpuBar.style.width = `${Math.min(h.cpu_usage, 100)}%`;
        
        const ramPct = h.ram_total > 0 ? (h.ram_used / h.ram_total) * 100 : 0;
        if(elHostRamVal) elHostRamVal.innerText = `${(h.ram_used/1024).toFixed(1)} GB`;
        if(elHostRamBar) elHostRamBar.style.width = `${Math.min(ramPct, 100)}%`;

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
        let statusClass = 'status-offline';
        let statusText = 'STOPPED';
        let badgesHtml = '';

        if (svc.health === 'Quarantined') {
            statusClass = 'status-quarantined';
            statusText = 'QUARANTINED';
            badgesHtml += `<span class="badge badge-quarantine" data-name="${svc.name}">⚠️ ${svc.violations?.length || 1} VIOLATIONS</span>`;
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
                fetch('/api/toggle-autopilot', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({service:'${svc.name}', enabled:${!svc.auto_pilot}})}).catch(e => console.error(e));
            `;
            
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
        const modal = document.getElementById('info-modal');
        if(!modal) return;

        modal.style.display = 'flex';
        
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
        document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));
        
        const vioTab = document.getElementById('tab-violations');
        if(vioTab) vioTab.style.display = 'none';

        const logsTabBtn = document.querySelector('.modal-tabs .tab-btn[data-target="view-logs"]');
        if(logsTabBtn) logsTabBtn.classList.add('active');
        
        const logsView = document.getElementById('view-logs');
        if(logsView) logsView.classList.add('active');
        
        this.startLogStream(id);
    },

    openViolationsModal(svc) {
        const modal = document.getElementById('info-modal');
        if(!modal) return;

        modal.style.display = 'flex';
        
        document.querySelectorAll('.modal-tabs .tab-btn').forEach(b => b.classList.remove('active'));
        document.querySelectorAll('.modal-view').forEach(v => v.classList.remove('active'));

        const vioTab = document.getElementById('tab-violations');
        const vioView = document.getElementById('view-violations');
        const vioOutput = document.getElementById('violations-output');

        if(vioTab) {
            vioTab.style.display = 'block';
            vioTab.classList.add('active');
        }
        if(vioView) vioView.classList.add('active');

        if(vioOutput) {
            let html = `<h3>🚫 QUARANTINE REPORT: ${svc.name}</h3><br>`;
            html += `<p>This service has been flagged by the Sentinel Governor for architectural compliance violations.</p><br><ul>`;
            (svc.violations || []).forEach(v => {
                html += `<li style="margin-bottom:10px;">${v}</li>`;
            });
            html += `</ul><br><p style="color:#666;">Action Required: Fix the environment variables (.env) or Docker configurations and restart the service.</p>`;
            vioOutput.innerHTML = html;
        }

        if (this.logSocket) this.logSocket.close(); 
    },

    startLogStream(id) {
        const logOutput = document.getElementById('log-output');
        if(!logOutput) return;

        logOutput.innerHTML = '';
        if (this.logSocket) this.logSocket.close();
        
        this.logSocket = new WebSocket(`ws://${window.location.host}/ws/logs/${id}`);
        this.logSocket.onmessage = (e) => {
            logOutput.innerHTML += e.data;
            const logView = document.getElementById('view-logs');
            if(logView) logView.scrollTop = logView.scrollHeight;
        };
    },

    async loadInspect(id) {
        const inspOut = document.getElementById('inspect-output');
        if(!inspOut) return;

        inspOut.innerText = "Scanning Docker API...";
        try {
            const res = await fetch(`/api/service/${id}/inspect`);
            const data = await res.json();
            const clean = {
                Id: data.Id, Created: data.Created, State: data.State,
                Config: { Image: data.Config.Image, Env: data.Config.Env },
                Network: data.NetworkSettings.Networks
            };
            inspOut.innerText = JSON.stringify(clean, null, 2);
        } catch(e) { inspOut.innerText = "Error: " + e; }
    },

    hideModal() {
        const modal = document.getElementById('info-modal');
        if(modal) modal.style.display = 'none';
        if (this.logSocket) this.logSocket.close();
    },

    updateConnectionStatus(isOnline) {
        const statuses = [
            document.getElementById('conn-status'),
            document.getElementById('conn-status-mobile')
        ];
        statuses.forEach(st => {
            if (st) {
                st.innerText = isOnline ? "● CLUSTER LINK ACTIVE" : "● OFFLINE";
                st.className = `conn-status ${isOnline ? 'online' : 'offline'}`;
            }
        });
    }
};

// Global Exposure for event handlers
window.Store = Store; 
window.ui = ui;

// Boot
ui.init(); 

new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    ui.updateConnectionStatus(true);
    if (msg.type === 'cluster_update') {
        Store.dispatch('CLUSTER_UPDATE', msg.data);
    }
}, (isOnline) => {
    ui.updateConnectionStatus(isOnline);
}).connect();