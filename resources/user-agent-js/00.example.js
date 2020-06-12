// Keep files in this directory which you would like executed before
// any other script when servo is run with `--userscripts`
// Files are sorted alphabetically. When committing polyfills
// order them with numbers, e.g. `01.innerhtml.js` will be executed before
// `05.jquery.js`
onunhandledrejection = function(e) { console.error("XXX promise rejection: " + e.reason); }
window.RTCDataChannelEvent = {};
window.RTCPeerConnection.prototype.createDataChannel = function() {
    return {
        addEventListener: function() {},
        removeEventListener: function() {},
        readyState: "closed",
    };
}
window.console.group = function() {}
window.console.groupEnd = function() {}
window.MediaDevices.prototype.enumerateDevices = function() {
    return Promise.resolve([]);
};
