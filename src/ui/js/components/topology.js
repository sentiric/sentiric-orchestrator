// src/ui/js/components/topology.js

export class TopologyMap {
    constructor(containerId) {
        this.container = document.getElementById(containerId);
        this.network = null;
        this.nodes = new vis.DataSet();
        this.edges = new vis.DataSet();
        this.isDrawn = false;

        this.colors = {
            expected: { bg: '#161b22', border: '#30363d', font: '#666' }, // Gri (Çalışmıyor ama beklenen)
            healthy: { bg: 'rgba(0, 255, 157, 0.1)', border: '#00ff9d', font: '#fff' }, // Yeşil (Sağlıklı)
            warning: { bg: 'rgba(210, 153, 34, 0.1)', border: '#d29922', font: '#fff' }, // Turuncu (CPU/RAM Yüksek)
            critical: { bg: 'rgba(248, 81, 73, 0.1)', border: '#f85149', font: '#fff' } // Kırmızı (Çökmüş)
        };
    }

    async draw() {
        if (this.isDrawn) return;

        try {
            const res = await fetch('/api/topology');
            const data = await res.json();

            // Node'ları vis.js formatına çevir
            const visNodes = data.nodes.map(n => ({
                id: n.id,
                label: n.label,
                group: n.group,
                shape: 'box',
                color: { background: this.colors.expected.bg, border: this.colors.expected.border },
                font: { color: this.colors.expected.font, face: 'JetBrains Mono', size: 12 },
                borderWidth: 2,
                shadow: true
            }));

            // Edge'leri vis.js formatına çevir
            const visEdges = data.edges.map(e => ({
                from: e.from,
                to: e.to,
                label: e.label,
                font: { align: 'middle', color: '#666', size: 10, face: 'Inter' },
                arrows: 'to',
                color: { color: '#333', highlight: '#00ff9d' },
                dashes: e.dashes
            }));

            this.nodes.add(visNodes);
            this.edges.add(visEdges);

            const options = {
                layout: {
                    hierarchical: {
                        direction: 'UD', // Yukardan aşağıya
                        sortMethod: 'directed',
                        nodeSpacing: 150,
                        levelSeparation: 150
                    }
                },
                physics: false, // Hierarchical layout'ta physics kapalı kalmalı
                interaction: {
                    hover: true,
                    zoomView: true,
                    dragView: true
                }
            };

            this.network = new vis.Network(this.container, { nodes: this.nodes, edges: this.edges }, options);

            // Tıklayınca Inspector Modal'ı aç
            this.network.on("click", (params) => {
                if (params.nodes.length > 0) {
                    const nodeId = params.nodes[0];
                    // Eğer o node sistemde çalışıyorsa (Listede var ise) Info modalını aç
                    const activeSvc = window.nexus && window.nexus.serviceAction ? true : false;
                    if(activeSvc) {
                         // DOM'daki kart üzerinden short_id'yi bulmamız lazım, 
                         // şimdilik ismini parametre geçerek inspector'a paslıyoruz.
                         console.log("Inspect Service:", nodeId);
                    }
                }
            });

            this.isDrawn = true;
        } catch (e) {
            console.error("Failed to load topology:", e);
        }
    }

    // Gelen canlı Docker verisiyle node renklerini günceller
    updateLiveState(liveServices) {
        if (!this.isDrawn) return;

        const updates = [];
        
        // Mevcut tüm node'ları gez
        this.nodes.get().forEach(node => {
            // Canlı veride bu node (servis) var mı?
            const liveSvc = liveServices.find(s => s.name === node.id);

            let newColor = this.colors.expected; // Varsayılan: Yok/Çökmüş

            if (liveSvc) {
                const isUp = liveSvc.status.toLowerCase().includes('up');
                if (isUp) {
                    // CPU veya RAM yüksekse Warning (Turuncu)
                    if (liveSvc.cpu_usage > 80 || (liveSvc.mem_usage > 2000)) {
                        newColor = this.colors.warning;
                    } else {
                        newColor = this.colors.healthy; // Her şey yolunda (Yeşil)
                    }
                } else {
                    newColor = this.colors.critical; // Servis stop edilmiş veya restart atıyor (Kırmızı)
                }
            } else {
                 newColor = this.colors.critical; // Beklenen mimaride var ama sistemde yok! (Kayıp)
            }

            // Sadece renk değiştiyse update at (Performans)
            if (node.color.border !== newColor.border) {
                updates.push({
                    id: node.id,
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