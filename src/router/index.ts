import { createRouter, createWebHashHistory } from 'vue-router'

// Tauri 使用 file:// 协议，必须用 hash 路由
const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/home' },
    {
      path: '/home',
      name: 'home',
      component: () => import('../pages/HomePage.vue'),
    },
    {
      path: '/search',
      name: 'search',
      component: () => import('../pages/SearchPage.vue'),
    },
    {
      path: '/feed/:id',
      name: 'feed',
      component: () => import('../pages/FeedDetailPage.vue'),
    },
    {
      path: '/settings',
      name: 'settings',
      component: () => import('../pages/SettingsPage.vue'),
    },
  ],
})

export default router
