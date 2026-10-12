// Ultra-simple badge selection script - NO COMPLEXITY
console.log('script.js loaded');

let selectedModel = 'minimax';

function selectModel(model) {
  console.log('SELECT MODEL:', model);
  selectedModel = model;
  
  // Update debug text
  const debugText = document.getElementById('debug-text');
  if (debugText) {
    debugText.textContent = 'Selected: ' + model;
    debugText.style.color = model === 'big-pickle' ? '#ff6b6b' : 
                                   model === 'nvidia' ? '#4caf50' : 
                                   '#ffc107';
  }
  
  // Remove all selected classes
  document.querySelectorAll('.model-badge').forEach(badge => {
    badge.classList.remove('selected');
  });
  
  // Add selected class to clicked badge
  const selectedBadge = document.querySelector('[data-model="' + model + '"]');
  if (selectedBadge) {
    selectedBadge.classList.add('selected');
    console.log('BADGE SELECTED:', selectedBadge);
  }
  
  // Store in localStorage
  localStorage.setItem('selectedModel', model);
  console.log('SAVED TO STORAGE:', model);
}

// Wait for DOM and setup
window.addEventListener('DOMContentLoaded', function() {
  console.log('DOM READY');
  
  // Setup click handlers with error checking
  const badges = ['big-pickle', 'nvidia', 'minimax'];
  
  badges.forEach(modelId => {
    const badge = document.getElementById('badge-' + modelId);
    if (badge) {
      console.log('SETTING UP BADGE:', modelId, badge);
      
      badge.onclick = function() {
        console.log('BADGE CLICKED:', modelId);
        selectModel(modelId);
      };
      
      console.log('BADGE ONCLICK SET:', badge.onclick);
    } else {
      console.error('BADGE NOT FOUND:', modelId);
    }
  });
  
  // Load saved model
  const saved = localStorage.getItem('selectedModel');
  if (saved) {
    console.log('LOADING SAVED MODEL:', saved);
    selectModel(saved);
  }
  
  console.log('SETUP COMPLETE');
});

// Global test function
window.testBadges = function() {
  console.log('=== TESTING ALL BADGES ===');
  document.getElementById('badge-bigpickle')?.click();
  document.getElementById('badge-kimi25')?.click();
  document.getElementById('badge-minimax25')?.click();
};
