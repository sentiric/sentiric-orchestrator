// src/ui/js/store.js
export const Store = {
    state: {
        cluster: {},
        selectedNode: null,
        localNodeName: null,
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
            case 'TOGGLE_AP_OPTIMISTIC':
                const { node, service, enabled } = payload;
                if (this.state.cluster[node]) {
                    const svc = this.state.cluster[node].services.find(s => s.name === service);
                    if (svc) svc.auto_pilot = enabled;
                }
                this.notify();
                break;
            case 'UPDATE_PROGRESS': // [YENİ]
                if (this.state.cluster[this.state.localNodeName]) {
                    const svcInfo = this.state.cluster[this.state.localNodeName].services.find(s => s.name === payload.service);
                    if (svcInfo) svcInfo.update_progress = payload.progress;
                }
                this.notify();
                break;
        }
    },

    updateHistory(clusterData) {
        const MAX_HISTORY = 40; 
        
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