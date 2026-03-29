import { writable } from 'svelte/store';

// Define the auth store interface
interface AuthStore {
  token: string | null;
  user: any | null;
  isAuthenticated: boolean;
}

// Create the auth store
const { subscribe, set, update } = writable<AuthStore>({
  token: null,
  user: null,
  isAuthenticated: false
});

// Auth actions
async function login(username: string, password: string) {
  try {
    // In a real implementation, this would call the login API
    // For now, we'll simulate a successful login
    const mockToken = 'mock-jwt-token-' + Date.now();
    const mockUser = { id: 1, username, email: `${username}@example.com` };
    
    // Store token in localStorage for persistence
    localStorage.setItem('auth_token', mockToken);
    localStorage.setItem('user', JSON.stringify(mockUser));
    
    update(store => ({
      ...store,
      token: mockToken,
      user: mockUser,
      isAuthenticated: true
    }));
  } catch (error) {
    console.error('Login failed:', error);
    throw error;
  }
}

function logout() {
  // Clear token and user from localStorage
  localStorage.removeItem('auth_token');
  localStorage.removeItem('user');
  
  // Update store
  update(store => ({
    ...store,
    token: null,
    user: null,
    isAuthenticated: false
  }));
}

export { subscribe, login, logout };