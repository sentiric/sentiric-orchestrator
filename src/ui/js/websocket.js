export class WebSocketStream {
    constructor(url, onMessage) {
        this.url = url;
        this.onMessage = onMessage;
        this.conn = null;
    }

    connect() {
        this.conn = new WebSocket(this.url);
        this.conn.onopen = () => console.log("💠 Nexus Uplink Established");
        this.conn.onclose = () => {
            console.log("⚠️ Nexus Uplink Lost. Reconnecting...");
            setTimeout(() => this.connect(), 3000);
        };
        this.conn.onmessage = (e) => {
            try { this.onMessage(JSON.parse(e.data)); } 
            catch(err) { console.error("Parse Error", err); }
        };
    }
}