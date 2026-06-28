// Minimal progressive enhancement — no framework, no build step.

// Current year in the footer.
document.getElementById('year').textContent = new Date().getFullYear();

// Mobile nav toggle.
const toggle = document.querySelector('.nav-toggle');
const mobileNav = document.querySelector('.mobile-nav');
if (toggle && mobileNav) {
  toggle.addEventListener('click', () => {
    const open = mobileNav.hasAttribute('hidden');
    if (open) {
      mobileNav.removeAttribute('hidden');
    } else {
      mobileNav.setAttribute('hidden', '');
    }
    toggle.setAttribute('aria-expanded', String(open));
  });
  // Close the menu after tapping a link.
  mobileNav.querySelectorAll('a').forEach((a) =>
    a.addEventListener('click', () => {
      mobileNav.setAttribute('hidden', '');
      toggle.setAttribute('aria-expanded', 'false');
    })
  );
}

// Subtle fade-in for cards/steps as they enter the viewport.
const reveals = document.querySelectorAll('.reveal');
if ('IntersectionObserver' in window && reveals.length) {
  const io = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add('in');
          io.unobserve(entry.target);
        }
      });
    },
    { threshold: 0.12 }
  );
  reveals.forEach((el) => io.observe(el));
} else {
  reveals.forEach((el) => el.classList.add('in'));
}
