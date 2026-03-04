// src/ui/js/components/topology.js
export class TopologyMap {
    constructor(containerId) {
        this.container = document.getElementById(containerId);
        this.network = null;
        this.nodes = typeof vis !== 'undefined' ? new vis.DataSet() : null;
        this.edges = typeof vis !== 'undefined' ? new vis.DataSet() : null;
        this.isDrawn = false;
        this.physicsEnabled = true;

        // Profesyonel Renk Paleti
        this.colors = {
            expected: { bg: '#111827', border: '#334155', font: '#94a3b8' }, // Gri (Eksik)
            healthy:  { bg: '#022c22', border: '#10b981', font: '#a7f3d0' }, // Yeşil (Sağlıklı)
            warning:  { bg: '#451a03', border: '#f59e0b', font: '#fde047' }, // Turuncu (Yük Altında)
            critical: { bg: '#450a0a', border: '#ef4444', font: '#fecaca' }  // Kırmızı (Çökmüş)
        };

        this.bindEvents();
    }

    bindEvents() {
        const btnFreeze = document.getElementById('btn-freeze-physics');
        if (btnFreeze) {
            btnFreeze.onclick = () => {
                this.physicsEnabled = !this.physicsEnabled;
                if (this.network) {
                    this.network.setOptions({ physics: { enabled: this.physicsEnabled } });
                }
                btnFreeze.innerText = this.physicsEnabled ? "❄️ FREEZE LAYOUT" : "▶️ UNFREEZE LAYOUT";
                btnFreeze.style.borderColor = this.physicsEnabled ? "var(--border-light)" : "var(--accent-green)";
                btnFreeze.style.color = this.physicsEnabled ? "var(--text-muted)" : "var(--accent-green)";
            };
        }
    }

    async draw() {
        if (this.isDrawn || !this.nodes) return;

        try {
            const res = await fetch('/api/topology');
            const data = await res.json();

            const visNodes = data.nodes.map(n => ({
                id: n.id,
                label: n.label,
                group: n.group,
                shape: 'box',
                color: { background: this.colors.expected.bg, border: this.colors.expected.border },
                font: { color: this.colors.expected.font, face: 'JetBrains Mono', size: 13, multi: true, bold: '14px' },
                borderWidth: 2,
                shadow: { enabled: true, color: 'rgba(0,0,0,0.9)', size: 10, x: 0, y: 5 },
                margin: { top: 12, bottom: 12, left: 16, right: 16 }
            }));

            const visEdges = data.edges.map(e => ({
                from: e.from, to: e.to, label: e.label,
                font: { align: 'middle', color: '#64748b', size: 11, face: 'JetBrains Mono', background: '#0a0a0a' },
                arrows: { to: { enabled: true, scaleFactor: 0.6 } },
                color: { color: '#334155', highlight: '#10b981' }, 
                dashes: e.dashes,
                smooth: { type: 'continuous', forceDirection: 'none', roundness: 0.5 }
            }));

            this.nodes.add(visNodes);
            this.edges.add(visEdges);

            const options = {
                layout: { hierarchical: false },
                physics: {
                    enabled: this.physicsEnabled,
                    barnesHut: {
                        gravitationalConstant: -3000,
                        centralGravity: 0.3,
                        springLength: 200,
                        springConstant: 0.04,
                        damping: 0.09,
                        avoidOverlap: 0.1
                    },
                    maxVelocity: 50,
                    minVelocity: 0.1,
                    solver: 'barnesHut',
                    stabilization: { enabled: true, iterations: 200, updateInterval: 50 }
                },
                interaction: { hover: true, zoomView: true, dragView: true, dragNodes: true }
            };

            this.network = new vis.Network(this.container, { nodes: this.nodes, edges: this.edges }, options);
            
            this.network.once("stabilizationIterationsDone", () => {
                this.network.fit({ animation: { duration: 1000, easingFunction: 'easeInOutQuad' } });
                // Otomatik dondur (Titreşimi önlemek için)
                document.getElementById('btn-freeze-physics').click();
            });

            this.isDrawn = true;
        } catch (e) {
            console.error("Failed to load topology:", e);
        }
    }

    updateLiveState(globalClusterState) {
        if (!this.isDrawn || !this.nodes || !globalClusterState) return;

        const allLiveServices = [];
        Object.keys(globalClusterState).forEach(nodeName => {
            const nodeData = globalClusterState[nodeName];
            nodeData.services.forEach(svc => {
                allLiveServices.push({ ...svc, runningNode: nodeName });
            });
        });

        const updates = [];
        
        this.nodes.get().forEach(node => {
            // İsim eşleşmesi (Hardcode edilen ID ile Docker container adı eşleşmeli)
            const liveSvc = allLiveServices.find(s => s.name === node.id);

            let newColor = this.colors.expected;
            let newLabel = node.label.split('\n')[0] + '\n' + node.label.split('\n')[1];

            if (liveSvc) {
                const isUp = liveSvc.status.toLowerCase().includes('up');
                if (isUp) {
                    if (liveSvc.cpu_usage > 80 || (liveSvc.mem_usage > 2000)) newColor = this.colors.warning;
                    else newColor = this.colors.healthy;
                    
                    const shortNodeName = liveSvc.runningNode.substring(0, 15);
                    newLabel += `\n[${shortNodeName}]`;
                } else {
                    newColor = this.colors.critical;
                    newLabel += `\n[STOPPED]`;
                }
            } else {
                 newColor = this.colors.critical;
                 newLabel += `\n[MISSING]`;
            }

            if (node.color.border !== newColor.border || node.label !== newLabel) {
                updates.push({
                    id: node.id,
                    label: newLabel,
                    color: { background: newColor.bg, border: newColor.border },
                    font: { color: newColor.font }
                });
            }
        });

        if (updates.length > 0) {
            this.nodes.update(updates);
        }
    }
}