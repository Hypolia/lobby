package fr.hypolia.lobby.entities;

import org.bukkit.Location;
import org.bukkit.entity.EntityType;
import org.bukkit.entity.Player;
import org.bukkit.entity.Villager;

import java.util.UUID;

public abstract class AbstractNPC {
  private final Villager villager;
  private final String name;

  public AbstractNPC(String name, Location location) {
    this.name = name;

    villager = (Villager) location.getWorld().spawnEntity(location, EntityType.VILLAGER);
    villager.setCustomName(name);
    villager.setCustomNameVisible(true);
    villager.setAI(false);
    villager.setInvulnerable(true);
    villager.setCollidable(false);
  }

  public String getName() {
    return name;
  }

  public UUID getUniqueId() {
    return villager.getUniqueId();
  }

  public void remove() {
    villager.remove();
  }

  public abstract void handleClick(Player player);
}
