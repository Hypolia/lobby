package fr.hypolia.lobby.commands;

import fr.hypolia.lobby.Lobby;
import org.bukkit.Bukkit;
import org.bukkit.Location;
import org.bukkit.command.Command;
import org.bukkit.command.CommandExecutor;
import org.bukkit.command.CommandSender;
import org.bukkit.entity.EntityType;
import org.bukkit.entity.Player;
import org.bukkit.entity.Villager;

public class SpawnNPC implements CommandExecutor {
  @Override
  public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
    if (!(sender instanceof Player)) {
      sender.sendMessage("You must be a player to use this command");
      return false;
    }

    Player player = (Player) sender;

    Location location = player.getLocation();

    Villager npc = (Villager) player.getWorld().spawnEntity(location, EntityType.VILLAGER);

    npc.setCustomName("Mini-Jeux");
    npc.setCustomNameVisible(true);
    npc.setAI(false);

    Bukkit.getScheduler().runTaskLater(Lobby.getPlugin(Lobby.class), () -> {
      npc.setCollidable(false);
    }, 1L);

    player.sendMessage("PNJ Mini-Jeux créé!");
    return true;
  }
}
