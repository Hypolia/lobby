package fr.hypolia.lobby.managers;

import fr.hypolia.lobby.Lobby;
import fr.hypolia.lobby.entities.AbstractNPC;
import org.bukkit.Bukkit;
import org.bukkit.entity.Entity;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerInteractAtEntityEvent;
import org.bukkit.event.player.PlayerInteractEntityEvent;
import org.bukkit.event.player.PlayerInteractEvent;
import org.bukkit.inventory.EquipmentSlot;

import java.util.HashMap;
import java.util.Map;
import java.util.UUID;

public class NPCManager implements Listener {

  private final Map<UUID, AbstractNPC> npcMap = new HashMap<>();

  public NPCManager() {
    // Add NPCs
    Bukkit.getPluginManager().registerEvents(this, Lobby.getInstance());
  }

  public void registerNPC(AbstractNPC npc) {
    npcMap.put(npc.getUniqueId(), npc);
  }

  public void unregisterNPC(Entity entity) {
    npcMap.remove(entity.getUniqueId());
  }

  @EventHandler
  public void onNPCClick(PlayerInteractEntityEvent event) {
    // Vérifiez si l'interaction vient de la main princpale
    Bukkit.getLogger().info("Player clicked NPC with: " + event.getHand());

    Bukkit.getLogger().info("getRightClicked: " + event.getRightClicked());
    // detect if the player is clicking with the main hand
    if (event.getHand() != EquipmentSlot.HAND) {
      return;
    }

    Entity clickedEntity = event.getRightClicked();
    Player player = event.getPlayer();

    if (npcMap.containsKey(clickedEntity.getUniqueId())) {
      event.setCancelled(true);
      npcMap.get(clickedEntity.getUniqueId()).handleClick(player);
    }
  }
}
