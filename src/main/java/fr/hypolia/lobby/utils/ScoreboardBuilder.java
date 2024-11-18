package fr.hypolia.lobby.utils;

import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.scoreboard.DisplaySlot;
import org.bukkit.scoreboard.Objective;
import org.bukkit.scoreboard.Score;
import org.bukkit.scoreboard.Scoreboard;

import java.util.HashMap;
import java.util.Map;

public class ScoreboardBuilder {

  private final Scoreboard scoreboard;
  private final Objective objective;
  private final Map<Integer, String> lines;

  /**
   * Constructeur pour initialiser un scoreboard.
   *
   * @param title Titre du scoreboard.
   */
  public ScoreboardBuilder(String title) {
    this.scoreboard = Bukkit.getScoreboardManager().getNewScoreboard();
    this.objective = this.scoreboard.registerNewObjective("dummy", "dummy", title);
    this.objective.setDisplaySlot(DisplaySlot.SIDEBAR);
    this.lines = new HashMap<>();
  }

  /**
   * Définit une ligne dans le scoreboard.
   *
   * @param line   La position de la ligne (1 étant la plus haute).
   * @param text   Le texte à afficher sur cette ligne.
   * @return L'instance actuelle de ScoreboardBuilder.
   */
  public ScoreboardBuilder setLine(int line, String text) {
    this.lines.put(line, text);
    return this;
  }

  /**
   * Supprime une ligne spécifique.
   *
   * @param line La position de la ligne à supprimer.
   * @return L'instance actuelle de ScoreboardBuilder.
   */
  public ScoreboardBuilder removeLine(int line) {
    this.lines.remove(line);
    return this;
  }

  /**
   * Met à jour le contenu du scoreboard avec les lignes configurées.
   */
  private void updateScoreboard() {
    this.lines.forEach((line, text) -> {
      Score score = this.objective.getScore(text);
      score.setScore(line);
    });
  }

  /**
   * Applique le scoreboard à un joueur.
   *
   * @param player Le joueur qui recevra le scoreboard.
   */
  public void applyToPlayer(Player player) {
    this.updateScoreboard();
    player.setScoreboard(this.scoreboard);
  }

  /**
   * Récupère le scoreboard construit.
   *
   * @return Le scoreboard Bukkit.
   */
  public Scoreboard build() {
    this.updateScoreboard();
    return this.scoreboard;
  }
}