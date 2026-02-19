import { WebSocketStream } from './websocket.js';

const state = {
    services: [],
    nodes: []
};

const ui = {
    grid: document.getElementById('services-grid'),
    
    render() {
        if (!state.services.length) return;
        
        // Sorting: GPU services first, then alphabetical
        const sorted = [...state.services].sort((a, b) => {
            if (a.has_gpu && !b.has_gpu) return -1;
            if (!a.has_gpu && b.has_gpu) return 1;
            return a.name.localeCompare(b.name);
        });

        this.grid.innerHTML = sorted.map(svc => this.createCard(svc)).join('');
    },

    createCard(svc) {
        return `
            <div class="card">
                <div class="card-header">
                    <div>
                        <span class="card-title">${svc.name}</span>
                        ${svc.has_gpu ? '<span class="gpu-badge">GPU</span>' : ''}
                    </div>
                    <span style="font-size: 9px; color: ${svc.status.includes('Up') ? 'var(--color-accent)' : 'var(--color-error)'}; font-weight: bold;">
                        ${svc.status}
                    </span>
                </div>
                
                ${this.createBar('CPU', svc.cpu_usage, '%', 'cpu')}
                ${this.createBar('RAM', svc.mem_usage, 'MB', 'ram')}
                
                <div class="btn-group">
                    <button onclick="window.nexus.toggleAP('${svc.name}', ${!svc.auto_pilot})">
                        ${svc.auto_pilot ? '🟢 AUTO-PILOT' : '⚪ MANUAL'}
                    </button>
                    <button class="btn-primary" onclick="window.nexus.updateSvc('${svc.name}')">
                        FORCE UPDATE
                    </button>
                </div>
            </div>
        `;
    },

    createBar(label, val, unit, type) {
        // Simple scaling: CPU max 100, RAM max 2048 (visual)
        let pct = val;
        if(type === 'ram') pct = (val / 2048) * 100;
        if(pct > 100) pct = 100;

        return `
            <div class="bar-container">
                <div class="bar-label">
                    <span>${label}</span>
                    <span>${val.toFixed(1)}${unit}</span>
                </div>
                <div class="bar-track">
                    <div class="bar-fill ${type}" style="width: ${pct}%"></div>
                </div>
            </div>
        `;
    }
};

// Global Actions (Window'a bağlıyoruz ki HTML onclick çalışsın)
window.nexus = {
    async updateSvc(name) {
        if(!confirm(`Update ${name}? This will restart the container.`)) return;
        try {
            await fetch(`/api/update?service=${name}`, { method: 'POST' });
            alert('Update command sent.');
        } catch(e) { alert('Error: ' + e); }
    },

    async toggleAP(service, enabled) {
        await fetch('/api/toggle-autopilot', { 
            method: 'POST', 
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({service, enabled})
        });
    }
};

// Start Stream
new WebSocketStream(`ws://${window.location.host}/ws`, (msg) => {
    if (msg.type === 'services_update') {
        state.services = msg.data;
        ui.render();
    }
    if (msg.type === 'node_update') {
        document.getElementById('node-name').innerText = msg.data.name;
    }
}).connect();