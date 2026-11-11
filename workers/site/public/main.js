const time = new Intl.DateTimeFormat(undefined, {
	hour: "numeric",
	minute: "2-digit",
});
const clock = document.querySelector("[data-clock]");
const ends = document.querySelector("[data-ends]");
const tick = () => {
	const now = Date.now();
	clock.textContent = time.format(now);
	ends.textContent = `Ends at ${time.format(now + ends.dataset.ends * 1000)}`;
};
tick();
setInterval(tick, 15000);

const player = document.querySelector(".player");
let idle;
const wake = () => {
	player.classList.remove("idle");
	clearTimeout(idle);
	idle = setTimeout(() => player.classList.add("idle"), 3000);
};
player.addEventListener("pointermove", wake);
player.addEventListener("pointerdown", wake);
wake();
