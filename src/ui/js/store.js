// src/ui/js/store.js
export const Store = {
    state: {
        cluster: {},
        selectedNode: null,
        localNodeName: null,
        // Hafıza Sızıntısı Tespiti İçin (Memory Leak Radar)
        history: {} 
    },
    listeners: [],
    
    subscribe(callback) {
        this.listeners.push(callback);
    },

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
            // Optimistic UI Update for Auto-Pilot
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

    // Yeni: Trend çizgileri için son 30 veriyi tutar
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

    notify() {
        this.listeners.forEach(fn => fn(this.state));
    }
};