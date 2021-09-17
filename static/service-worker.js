const staticFiles = [
	// form
	'/',

	// websocket info
	'/wsinfo',

	// service worker and progressive web app
	'/service-worker.js',
	'/manifest.json',

	// static resources
	'/static/icons/lights-96-any.png',
	'/static/icons/lights-192-any.png',
	'/static/icons/lights-512-any.png',
	'/static/icons/lights-32.png',
	'/static/icons/lights-64.png',
	'/static/icons/lights-96.png',
	'/static/icons/lights-128.png',
	'/static/icons/lights-192.png',
	'/static/icons/lights-256.png',
	'/static/icons/lights-512.png',

	// color picker
	'/static/vendor/color-picker/color-picker.min.css',
	'/static/vendor/color-picker/color-picker.min.js',
];

self.addEventListener('install', (ev) => {
	ev.waitUntil((async () => {
		const cache = await caches.open('lights-static');
		await cache.addAll(staticFiles);
	})());
});

self.addEventListener('fetch', (ev) => {
	const url = new URL(ev.request.url);
	if (url.pathname === '/' || url.pathname.startsWith('/static/')) {
		ev.respondWith((async () => {
			return fetch(ev.request)
				.then(async (response) => {
					const cache = await caches.open('lights-static');
					cache.put(ev.request, response.clone());
					return response;
				})
				.catch(async () => {
					const cached = await caches.match(ev.request);
					return cached;
				});
		})());
	}
});
