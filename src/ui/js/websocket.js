// src/ui/js/websocket.js
export class WebSocketStream {
    constructor(url, onMessage, onStatusChange) {
        this.url = url;
        this.onMessage = onMessage;
        this.onStatusChange = onStatusChange;
        this.conn = null;
    }

    connect() {
        this.conn = new WebSocket(this.url);
        
        this.conn.onopen = () => {
            console.log("💠 Nexus Uplink Established");
            if(this.onStatusChange) this.onStatusChange(true);
        };
        
        this.conn.onclose = () => {
            console.log("⚠️ Nexus Uplink Lost. Reconnecting...");
            if(this.onStatusChange) this.onStatusChange(false);
            setTimeout(() => this.connect(), 3000);
        };
        
        this.conn.onmessage = (e) => {
            try { this.onMessage(JSON.parse(e.data)); } 
            catch(err) { console.error("Parse Error", err); }
        };
    }
}